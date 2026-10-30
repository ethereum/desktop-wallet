use alloy_primitives::{
    U256,
    utils::{format_units, parse_units},
};
use kohaku_tornadocash::{Asset, Pool};

use crate::vault::tornadocash::TornadoNote;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PoolSelection {
    Classic,
    #[default]
    IncludeExtended,
}

#[derive(Debug, thiserror::Error)]
pub enum AmountError {
    #[error("{0}")]
    Sponsor(String),
    #[error("amount must be greater than zero")]
    Zero,
    #[error("no pools for the selected asset on chain {chain_id}")]
    NoNativePools { chain_id: u64 },
    #[error("amount {amount} is not a sum of the selected pool denominations on chain {chain_id}")]
    NotADenominationSum { amount: U256, chain_id: u64 },
    #[error(
        "shield would create {count} notes; a single shield supports at most 5 notes. Choose larger denominations or split it into multiple shields"
    )]
    TooManyNotes { count: U256 },
    #[error("{amount} base units is not an enabled pool denomination on chain {chain_id}")]
    UnsupportedDenomination { amount: U256, chain_id: u64 },
    #[error("held notes cannot make {amount} wei exactly")]
    CannotMake { amount: U256 },
    #[error("held notes cannot cover {amount} wei")]
    CannotCover { amount: U256 },
    #[error("invalid decimal amount `{0}`")]
    Parse(String),
}

impl PoolSelection {
    #[must_use]
    pub fn includes(self, pool: &Pool) -> bool {
        self == Self::IncludeExtended || pool.canonical
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

pub fn deposit_pools_with_selection(
    amount: U256,
    chain_id: u64,
    selection: PoolSelection,
) -> Result<Vec<Pool>, AmountError> {
    deposit_pools_for_asset_with_selection(amount, chain_id, &Asset::ETH, selection)
}

pub fn deposit_pools_for_denominations(
    denominations: &[U256],
    chain_id: u64,
    selection: PoolSelection,
) -> Result<Vec<Pool>, AmountError> {
    deposit_pools_for_asset_denominations(denominations, chain_id, &Asset::ETH, selection)
}

pub fn deposit_pools_for_asset_denominations(
    denominations: &[U256],
    chain_id: u64,
    asset: &Asset,
    selection: PoolSelection,
) -> Result<Vec<Pool>, AmountError> {
    if denominations.is_empty() {
        return Err(AmountError::Zero);
    }
    if denominations.len() > 5 {
        return Err(AmountError::TooManyNotes {
            count: U256::from(denominations.len()),
        });
    }
    let pools = asset_pools(chain_id, asset, selection);
    denominations
        .iter()
        .map(|amount| {
            pools
                .iter()
                .find(|pool| U256::from(pool.amount_wei) == *amount)
                .cloned()
                .ok_or(AmountError::UnsupportedDenomination {
                    amount: *amount,
                    chain_id,
                })
        })
        .collect()
}

/// Splits an amount in base units using only this chain and asset.
pub fn deposit_pools_for_asset(
    amount: U256,
    chain_id: u64,
    asset: &Asset,
) -> Result<Vec<Pool>, AmountError> {
    deposit_pools_for_asset_with_selection(amount, chain_id, asset, PoolSelection::default())
}

pub fn deposit_pools_for_asset_with_selection(
    amount: U256,
    chain_id: u64,
    asset: &Asset,
    selection: PoolSelection,
) -> Result<Vec<Pool>, AmountError> {
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    let pools = asset_pools(chain_id, asset, selection);
    if pools.is_empty() {
        return Err(AmountError::NoNativePools { chain_id });
    }
    let mut remaining = amount;
    let mut chosen = Vec::new();
    let mut count = U256::ZERO;
    for pool in &pools {
        let denom = U256::from(pool.amount_wei);
        let pieces = remaining / denom;
        count += pieces;
        if count > U256::from(5) {
            return Err(AmountError::TooManyNotes { count });
        }
        for _ in 0..u64_from(pieces) {
            chosen.push(pool.clone());
        }
        remaining -= denom * pieces;
    }
    if !remaining.is_zero() {
        return Err(AmountError::NotADenominationSum { amount, chain_id });
    }
    Ok(chosen)
}

/// Notes whose denominations sum to `amount`, largest first.
///
/// Only unspent mined native ETH notes are eligible.
///
/// # Errors
/// Returns an error when the held notes cannot make `amount` exactly.
pub fn exact_notes(amount: U256, notes: &[TornadoNote]) -> Result<Vec<TornadoNote>, AmountError> {
    exact_notes_for_asset(amount, notes, &Asset::ETH)
}

/// Selects mined, unspent notes of exactly one asset. Callers must scope notes to their chain.
pub fn exact_notes_for_asset(
    amount: U256,
    notes: &[TornadoNote],
    asset: &Asset,
) -> Result<Vec<TornadoNote>, AmountError> {
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    let mut available: Vec<_> = notes
        .iter()
        .filter(|note| note.leaf_index.is_some() && !note.spent && &note.pool.asset == asset)
        .cloned()
        .collect();
    available.sort_by_key(|note| std::cmp::Reverse(note.pool.amount_wei));
    let mut best = None;
    select_notes(&available, 0, amount, false, &mut Vec::new(), &mut best);
    best.ok_or(AmountError::CannotMake { amount })
}

/// The smallest largest-first set of notes whose value is at least `amount`.
///
/// Whole denominations are taken largest first. A remainder takes the smallest held note that
/// covers it.
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
    parse_amount(amount, 18)
}

/// Parses a nonnegative decimal without silently rounding fractional base units.
pub fn parse_amount(amount: &str, decimals: u8) -> Result<U256, AmountError> {
    let text = amount.trim();
    let parts: Vec<_> = text.split('.').collect();
    if parts.len() > 2
        || parts[0].is_empty()
        || !parts[0].bytes().all(|b| b.is_ascii_digit())
        || parts.get(1).is_some_and(|part| {
            part.len() > usize::from(decimals)
                || part.is_empty()
                || !part.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(AmountError::Parse(amount.to_string()));
    }
    let parsed = parse_units(text, decimals).map_err(|_| AmountError::Parse(amount.to_string()))?;
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
    format_amount(amount, 18)
}

/// Renders base units with the selected asset decimals.
#[must_use]
pub fn format_amount(amount: U256, decimals: u8) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let Ok(formatted) = format_units(amount, decimals) else {
        return amount.to_string();
    };
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn asset_pools(chain_id: u64, asset: &Asset, selection: PoolSelection) -> Vec<Pool> {
    let mut pools: Vec<Pool> = Pool::POOLS
        .iter()
        .filter(|pool| {
            pool.chain_id == chain_id && &pool.asset == asset && selection.includes(pool)
        })
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
        if best.as_ref().is_none_or(|prior| {
            if cover {
                note_sum(chosen) < note_sum(prior)
                    || (note_sum(chosen) == note_sum(prior) && chosen.len() < prior.len())
            } else {
                chosen.len() < prior.len()
            }
        }) {
            *best = Some(chosen.clone());
        }
        return;
    }
    let Some(first) = notes.get(start) else {
        return;
    };
    if !cover
        && best
            .as_ref()
            .is_some_and(|prior| chosen.len() >= prior.len())
    {
        return;
    }
    if cover
        && best
            .as_ref()
            .is_some_and(|prior| note_sum(chosen) >= note_sum(prior))
    {
        return;
    }
    if !cover
        && best.as_ref().is_some_and(|prior| {
            remaining.div_ceil(U256::from(first.pool.amount_wei))
                > U256::from(prior.len() - chosen.len())
        })
    {
        return;
    }
    if notes[start..]
        .iter()
        .map(|note| U256::from(note.pool.amount_wei))
        .sum::<U256>()
        < remaining
    {
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

/// Resolves only the supported symbols, using the chain-specific pool catalog.
pub fn resolve_asset(symbol: &str, chain_id: u64) -> Result<Asset, String> {
    if !["ETH", "USDC", "USDT", "DAI", "WBTC"]
        .iter()
        .any(|s| symbol.eq_ignore_ascii_case(s))
    {
        return Err(format!(
            "unsupported token `{symbol}`; expected ETH, USDC, USDT, DAI, or WBTC"
        ));
    }
    Pool::POOLS
        .iter()
        .find(|pool| pool.chain_id == chain_id && pool.symbol().eq_ignore_ascii_case(symbol))
        .map(|pool| pool.asset.clone())
        .ok_or_else(|| format!("no {symbol} pools on chain {chain_id}"))
}

/// Selects one chain/asset, placing an ETH paymaster note first. Every ERC-20
/// withdrawal spends a separate ETH sponsor; its remainder stays on the fresh account.
pub fn withdrawal_notes(
    amount: U256,
    notes: &[TornadoNote],
    asset: &Asset,
    chain_id: u64,
) -> Result<Vec<TornadoNote>, AmountError> {
    let held: Vec<_> = notes
        .iter()
        .filter(|note| note.pool.chain_id == chain_id)
        .cloned()
        .collect();
    let mut selected = exact_notes_for_asset(amount, &held, asset)?;
    if asset == &Asset::ETH {
        let index = selected
            .iter()
            .position(|note| note.pool.paymaster.is_some())
            .ok_or_else(|| {
                AmountError::Sponsor("the selected ETH notes have no paymaster".into())
            })?;
        selected.swap(0, index);
    } else {
        let sponsor = held.into_iter().filter(|note| !note.spent && note.leaf_index.is_some() && note.pool.asset == Asset::ETH && note.pool.paymaster.is_some())
            .min_by_key(|note| note.pool.amount_wei)
            .ok_or_else(|| AmountError::Sponsor("every ERC-20 withdrawal requires an additional mined shielded ETH note with a paymaster to sponsor gas".into()))?;
        selected.insert(0, sponsor);
    }
    Ok(selected)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn wei(eth_tenths: u128) -> U256 {
        U256::from(eth_tenths) * U256::from(10_u128.pow(17))
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
    fn canonical_selection_rejects_extended_denominations() {
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

    #[test]
    fn cover_prefers_least_change_and_exact_backtracks() {
        let notes = vec![
            held(Pool::ETHEREUM_ETHER_1, 0),
            held(Pool::ETHEREUM_ETHER_03, 1),
            held(Pool::ETHEREUM_ETHER_01, 2),
            held(Pool::ETHEREUM_ETHER_001, 3),
            held(Pool::ETHEREUM_ETHER_001, 4),
        ];
        let covered = cover_notes(parse_eth("0.4111").expect("amount"), &notes).expect("cover");
        assert_eq!(note_sum(&covered), parse_eth("0.42").expect("sum"));
        let exact = exact_notes(parse_eth("0.42").expect("amount"), &notes).expect("exact");
        assert_eq!(exact.len(), 4);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod token_tests {
    use super::*;
    #[test]
    fn catalog_selection_units_and_denominations_cover_supported_tokens() {
        for (symbol, chain, amount, decimals, count) in [
            ("USDC", 1, "1100", 6, 2),
            ("usdt", 1, "100", 6, 1),
            ("DAI", 1, "11100", 18, 3),
            ("WBTC", 1, "1.1", 8, 2),
            ("dai", 11_155_111, "100", 18, 1),
            ("ETH", 1, "1.1", 18, 2),
        ] {
            let asset = resolve_asset(symbol, chain).expect("supported asset");
            assert_eq!(asset.decimals(), decimals);
            let raw = parse_amount(amount, decimals).expect("amount");
            assert_eq!(format_amount(raw, decimals), amount);
            let pools = deposit_pools_for_asset(raw, chain, &asset).expect("pools");
            assert_eq!(pools.len(), count);
            assert!(
                pools
                    .iter()
                    .all(|p| p.asset == asset && p.chain_id == chain)
            );
            assert_eq!(
                pools.iter().map(|p| U256::from(p.amount_wei)).sum::<U256>(),
                raw
            );
        }
        for symbol in ["USDC", "USDT", "WBTC", "DOGE"] {
            assert!(resolve_asset(symbol, 11_155_111).is_err());
        }
        for value in ["-1", "+1", "1e6", ".1", "1.", "1.0000001", "1..0"] {
            assert!(parse_amount(value, 6).is_err(), "accepted {value}");
        }
        assert_eq!(
            parse_amount("0.00000001", 8).expect("one satoshi"),
            U256::from(1)
        );
        assert!(deposit_pools_for_asset(U256::from(1), 1, &Asset::ETHEREUM_USDC).is_err());
    }
    fn note(pool: Pool, nonce: u64) -> TornadoNote {
        TornadoNote {
            pool,
            nonce,
            leaf_index: Some(0),
            spent: false,
        }
    }
    #[test]
    fn withdrawals_do_not_mix_assets_chains_pending_or_spent_notes() {
        let mut pending = note(Pool::ETHEREUM_USDC_100, 3);
        pending.leaf_index = None;
        let mut spent = note(Pool::ETHEREUM_USDC_100, 4);
        spent.spent = true;
        let held = [
            note(Pool::ETHEREUM_USDC_100, 0),
            note(Pool::ETHEREUM_USDT_100, 1),
            note(Pool::SEPOLIA_DAI_100, 2),
            pending,
            spent,
            note(Pool::ETHEREUM_ETHER_01, 5),
        ];
        let amount = parse_amount("100", 6).expect("100 USDC");
        let selected = withdrawal_notes(amount, &held, &Asset::ETHEREUM_USDC, 1)
            .expect("ETH sponsor and token note");
        assert_eq!(selected, vec![held[5].clone(), held[0].clone()]);
        assert!(withdrawal_notes(amount * U256::from(2), &held, &Asset::ETHEREUM_USDC, 1).is_err());
        assert!(withdrawal_notes(amount, &held, &Asset::ETHEREUM_USDC, 11_155_111).is_err());
    }
    #[test]
    fn every_token_requires_eth_even_when_its_pool_has_a_paymaster() {
        for pool in [
            Pool::ETHEREUM_USDC_100,
            Pool::ETHEREUM_USDT_100,
            Pool::ETHEREUM_DAI_100,
            Pool::ETHEREUM_DAI_10000,
            Pool::ETHEREUM_WBTC_01,
            Pool::SEPOLIA_DAI_100,
        ] {
            let token = note(pool, 0);
            let mut eth = note(
                if token.pool.chain_id == 1 {
                    Pool::ETHEREUM_ETHER_01
                } else {
                    Pool::SEPOLIA_ETHER_01
                },
                1,
            );
            let amount = U256::from(token.pool.amount_wei);
            let asset = &token.pool.asset;
            let chain = token.pool.chain_id;
            assert!(withdrawal_notes(amount, std::slice::from_ref(&token), asset, chain).is_err());
            assert_eq!(
                withdrawal_notes(amount, &[token.clone(), eth.clone()], asset, chain)
                    .expect("ETH-sponsored token withdrawal"),
                vec![eth.clone(), token.clone()]
            );
            eth.spent = true;
            assert!(withdrawal_notes(amount, &[token.clone(), eth.clone()], asset, chain).is_err());
            eth.spent = false;
            eth.leaf_index = None;
            assert!(withdrawal_notes(amount, &[token.clone(), eth.clone()], asset, chain).is_err());
            eth.leaf_index = Some(0);
            eth.pool = if chain == 1 {
                Pool::SEPOLIA_ETHER_01
            } else {
                Pool::ETHEREUM_ETHER_01
            };
            assert!(withdrawal_notes(amount, &[token.clone(), eth], asset, chain).is_err());
        }
    }
}
