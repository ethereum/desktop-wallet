use alloy_primitives::{
    U256,
    utils::{format_units, parse_units},
};
use kohaku_tornadocash::{Asset, Pool, PoolPackingError, PoolSelection, PoolSet};

use crate::vault::tornadocash::TornadoNote;

#[derive(Debug, thiserror::Error)]
pub enum AmountError {
    #[error("amount must be greater than zero")]
    Zero,
    #[error("no native ETH pools on chain {chain_id}")]
    NoNativePools { chain_id: u64 },
    #[error("amount {amount} is not a sum of the native pool denominations on chain {chain_id}")]
    NotADenominationSum { amount: U256, chain_id: u64 },
    #[error("held notes cannot make {amount} wei exactly")]
    CannotMake { amount: U256 },
    #[error("held notes cannot cover {amount} wei")]
    CannotCover { amount: U256 },
    #[error("invalid ETH amount `{0}`")]
    Parse(String),
}

impl From<PoolPackingError> for AmountError {
    fn from(error: PoolPackingError) -> Self {
        match error {
            PoolPackingError::Zero => Self::Zero,
            PoolPackingError::NoNativePools { chain_id } => Self::NoNativePools { chain_id },
            PoolPackingError::NotADenominationSum { amount, chain_id } => {
                Self::NotADenominationSum { amount, chain_id }
            }
        }
    }
}

/// Splits `amount` into native ETH pool deposits, largest denomination first.
///
/// # Errors
/// Returns an error when `amount` is zero, the chain has no native pools, or `amount` is not a
/// sum of those denominations.
pub fn deposit_pools(amount: U256, chain_id: u64) -> Result<Vec<Pool>, AmountError> {
    deposit_pools_with_selection(amount, chain_id, PoolSelection::default())
}

/// Packs an amount using the enabled pool families, with the fewest deposits.
/// The catalog's native denominations form the canonical 1/3/10 decimal coin system.
pub fn deposit_pools_with_selection(
    amount: U256,
    chain_id: u64,
    selection: PoolSelection,
) -> Result<Vec<Pool>, AmountError> {
    PoolSet::new(chain_id)
        .with_selection(selection)
        .deposit_pools(amount)
        .map_err(Into::into)
}

/// Notes whose denominations sum to `amount`, largest first.
///
/// Only unspent mined native ETH notes are eligible.
///
/// # Errors
/// Returns an error when the held notes cannot make `amount` exactly.
pub fn exact_notes(amount: U256, notes: &[TornadoNote]) -> Result<Vec<TornadoNote>, AmountError> {
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    let available = eligible(notes);
    let mut best = None;
    select_notes(&available, 0, amount, false, &mut Vec::new(), &mut best);
    best.ok_or(AmountError::CannotMake { amount })
}

/// The fewest notes covering `amount`, preferring the least excess value on ties.
/// Returns the chosen notes largest first.
///
/// # Errors
/// Returns an error when the held notes sum to less than `amount`.
pub fn cover_notes(amount: U256, notes: &[TornadoNote]) -> Result<Vec<TornadoNote>, AmountError> {
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    let available = eligible(notes);
    let mut best = None;
    select_notes(&available, 0, amount, true, &mut Vec::new(), &mut best);
    best.ok_or(AmountError::CannotCover { amount })
}

/// Parses a decimal ETH amount into wei.
///
/// # Errors
/// Returns an error when `amount` is not a non-negative decimal with at most 18 fractional digits.
pub fn parse_eth(amount: &str) -> Result<U256, AmountError> {
    let parsed =
        parse_units(amount.trim(), 18).map_err(|_| AmountError::Parse(amount.to_string()))?;
    Ok(From::from(parsed))
}

/// Parses an integer wei amount.
///
/// # Errors
/// Returns an error when `amount` is not a non-negative decimal integer.
pub fn parse_wei(amount: &str) -> Result<U256, AmountError> {
    let text = amount.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AmountError::Parse(amount.to_string()));
    }
    text.parse()
        .map_err(|_| AmountError::Parse(amount.to_string()))
}

/// Renders `amount` wei as a decimal ETH string.
#[must_use]
pub fn format_eth(amount: U256) -> String {
    let Ok(formatted) = format_units(amount, 18) else {
        return amount.to_string();
    };
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
fn native_pools(chain_id: u64, selection: PoolSelection) -> Vec<Pool> {
    let mut pools: Vec<Pool> = Pool::POOLS
        .iter()
        .filter(|pool| pool.chain_id == chain_id && selection.includes(pool) && is_native_eth(pool))
        .cloned()
        .collect();
    pools.sort_by_key(|pool| std::cmp::Reverse(pool.amount_wei));
    pools
}

fn eligible(notes: &[TornadoNote]) -> Vec<TornadoNote> {
    let mut notes: Vec<TornadoNote> = notes
        .iter()
        .filter(|note| note.leaf_index.is_some() && !note.spent && is_native_eth(&note.pool))
        .cloned()
        .collect();
    notes.sort_by_key(|note| std::cmp::Reverse(note.pool.amount_wei));
    notes
}

fn select_notes(
    notes: &[TornadoNote],
    start: usize,
    remaining: U256,
    cover: bool,
    chosen: &mut Vec<TornadoNote>,
    best: &mut Option<Vec<TornadoNote>>,
) {
    if remaining.is_zero() {
        if best.as_ref().is_none_or(|best| {
            chosen.len() < best.len()
                || (chosen.len() == best.len() && note_sum(chosen) < note_sum(best))
        }) {
            *best = Some(chosen.clone());
        }
        return;
    }
    let Some(first) = notes.get(start) else {
        return;
    };
    if best.as_ref().is_some_and(|best| chosen.len() >= best.len()) {
        return;
    }
    // Even using only the largest remaining note, this many notes are necessary.
    let lower_bound = remaining.div_ceil(U256::from(first.pool.amount_wei));
    if best
        .as_ref()
        .is_some_and(|best| lower_bound > U256::from(best.len() - chosen.len()))
    {
        return;
    }
    let total: U256 = notes[start..]
        .iter()
        .map(|note| U256::from(note.pool.amount_wei))
        .sum();
    if total < remaining {
        return;
    }
    let denom = first.pool.amount_wei;
    let end = start
        + notes[start..]
            .iter()
            .take_while(|note| note.pool.amount_wei == denom)
            .count();
    let unit = U256::from(denom);
    let want = remaining / unit + U256::from(u8::from(cover && remaining % unit != U256::ZERO));
    let max = usize::try_from(u64_from(want))
        .unwrap_or(usize::MAX)
        .min(end - start);
    let original = chosen.len();
    for count in (0..=max).rev() {
        chosen.extend_from_slice(&notes[start..start + count]);
        select_notes(
            notes,
            end,
            remaining.saturating_sub(unit * U256::from(count)),
            cover,
            chosen,
            best,
        );
        chosen.truncate(original);
    }
}

fn note_sum(notes: &[TornadoNote]) -> U256 {
    notes
        .iter()
        .map(|note| U256::from(note.pool.amount_wei))
        .sum()
}

fn is_native_eth(pool: &Pool) -> bool {
    matches!(&pool.asset, Asset::Native { symbol, .. } if symbol == "eth")
}

fn u64_from(value: U256) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn wei(eth_tenths: u128) -> U256 {
        U256::from(eth_tenths) * U256::from(10_u128.pow(17))
    }

    #[test]
    fn default_packing_includes_extended_and_is_minimal_on_both_chains() {
        for chain in [1, 11_155_111] {
            let amount = parse_eth("33.34").expect("amount");
            assert!(deposit_pools_with_selection(amount, chain, PoolSelection::Classic).is_err());
            let pools = deposit_pools(amount, chain).expect("default pack");
            assert_eq!(
                pools.iter().map(Pool::amount).collect::<Vec<_>>(),
                ["30", "3", "0.3", "0.03", "0.01"]
            );
            assert!(pools.iter().all(Pool::is_extended));
            assert!(deposit_pools(amount + U256::from(1), chain).is_err());
        }
    }

    #[test]
    fn packing_matches_dynamic_programming_minimum() {
        // Compare greedy packing against an independent optimum through 4 ETH.
        for chain in [1, 11_155_111] {
            for selection in [PoolSelection::Classic, PoolSelection::IncludeExtended] {
                let pools = native_pools(chain, selection);
                let coins: Vec<usize> = pools
                    .iter()
                    .map(|pool| (pool.amount_wei / 10_u128.pow(16)) as usize)
                    .collect();
                let mut minimum = vec![usize::MAX; 401];
                minimum[0] = 0;
                for amount in 1..minimum.len() {
                    for coin in &coins {
                        if *coin <= amount && minimum[amount - coin] != usize::MAX {
                            minimum[amount] = minimum[amount].min(minimum[amount - coin] + 1);
                        }
                    }
                    let packed = deposit_pools_with_selection(
                        U256::from(amount) * U256::from(10_u128.pow(16)),
                        chain,
                        selection,
                    );
                    if minimum[amount] == usize::MAX {
                        assert!(packed.is_err());
                    } else {
                        assert_eq!(packed.expect("pack").len(), minimum[amount]);
                    }
                }
            }
        }
    }

    #[test]
    fn exact_selection_backtracks_when_greedy_would_fail() {
        let mut notes = vec![held(Pool::ETHEREUM_ETHER_01, 0)];
        notes.extend((1..=4).map(|nonce| held(Pool::ETHEREUM_ETHER_003, nonce)));
        let chosen =
            exact_notes(parse_eth("0.12").expect("amount"), &notes).expect("four .03 notes");
        assert_eq!(chosen.len(), 4);
        assert!(
            chosen
                .iter()
                .all(|note| note.pool == Pool::ETHEREUM_ETHER_003)
        );
    }

    #[test]
    fn covering_minimizes_count_then_excess_and_ignores_ineligible_notes() {
        let mut notes = vec![
            held(Pool::ETHEREUM_ETHER_1, 0),
            held(Pool::ETHEREUM_ETHER_03, 1),
            held(Pool::ETHEREUM_ETHER_01, 2),
        ];
        notes.extend((3..=6).map(|nonce| held(Pool::ETHEREUM_ETHER_003, nonce)));
        let amount = parse_eth("0.12").expect("amount");
        let cover = cover_notes(amount, &notes).expect("cover");
        assert_eq!(cover.len(), 1);
        assert_eq!(cover[0].pool, Pool::ETHEREUM_ETHER_03);
        notes[0].spent = true;
        notes[1].leaf_index = None;
        assert_eq!(exact_notes(amount, &notes).expect("exact").len(), 4);
        assert!(cover_notes(parse_eth("1").expect("amount"), &notes).is_err());
    }

    #[test]
    fn held_note_selection_matches_brute_force() {
        let notes = vec![
            held(Pool::ETHEREUM_ETHER_03, 0),
            held(Pool::ETHEREUM_ETHER_01, 1),
            held(Pool::ETHEREUM_ETHER_003, 2),
            held(Pool::ETHEREUM_ETHER_003, 3),
            held(Pool::ETHEREUM_ETHER_001, 4),
        ];
        for cents in 1..=50 {
            let amount = U256::from(cents) * U256::from(10_u128.pow(16));
            let mut exact = None;
            let mut cover = None;
            for mask in 1..(1 << notes.len()) {
                let subset: Vec<_> = notes
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, note)| note.clone())
                    .collect();
                let key = (subset.len(), note_sum(&subset));
                if key.1 == amount && exact.is_none_or(|best| key < best) {
                    exact = Some(key);
                }
                if key.1 >= amount && cover.is_none_or(|best| key < best) {
                    cover = Some(key);
                }
            }
            assert_eq!(
                exact_notes(amount, &notes)
                    .ok()
                    .map(|chosen| (chosen.len(), note_sum(&chosen))),
                exact
            );
            assert_eq!(
                cover_notes(amount, &notes)
                    .ok()
                    .map(|chosen| (chosen.len(), note_sum(&chosen))),
                cover
            );
        }
    }

    #[test]
    fn one_point_one_eth_is_a_one_and_a_tenth() {
        let pools = deposit_pools(wei(11), 1).expect("1.1 splits");
        let amounts: Vec<u128> = pools.iter().map(|pool| pool.amount_wei).collect();
        assert_eq!(amounts, vec![10_u128.pow(18), 10_u128.pow(17)]);
    }

    #[test]
    fn a_remainder_is_rejected() {
        assert!(deposit_pools(wei(11) + U256::from(1_u8), 1).is_err());
    }

    #[test]
    fn parses_decimal_eth() {
        assert_eq!(parse_eth("0.1").expect("0.1"), wei(1));
        assert_eq!(parse_eth("1").expect("1"), wei(10));
    }

    #[test]
    fn canonical_only_rejects_extended_amounts() {
        assert!(
            deposit_pools_with_selection(
                parse_eth("0.05").expect("0.05"),
                1,
                PoolSelection::Classic
            )
            .is_err()
        );
        assert!(
            deposit_pools_with_selection(
                parse_eth("1.15").expect("1.15"),
                1,
                PoolSelection::Classic
            )
            .is_err()
        );
        assert!(deposit_pools(U256::from(1_u8), 1).is_err());
    }

    #[test]
    fn raw_wei_is_an_integer() {
        assert_eq!(parse_wei("100000000000000000").expect("0.1 ETH"), wei(1));
        assert!(parse_wei("1.1").is_err());
        assert!(parse_wei("0x10").is_err());
    }

    fn held(pool: Pool, nonce: u64) -> TornadoNote {
        TornadoNote {
            pool,
            nonce,
            leaf_index: Some(0),
            spent: false,
        }
    }

    #[test]
    fn cover_takes_a_tenth_after_a_whole_ether() {
        let notes = vec![
            held(Pool::ETHEREUM_ETHER_1, 0),
            held(Pool::ETHEREUM_ETHER_01, 1),
        ];
        let covered = cover_notes(wei(10) + wei(1) / U256::from(2_u8), &notes).expect("cover");
        let sum: U256 = covered
            .iter()
            .map(|note| U256::from(note.pool.amount_wei))
            .sum();
        assert!(sum >= wei(10) + wei(1) / U256::from(2_u8));
        assert_eq!(covered.len(), 2);
    }
}
