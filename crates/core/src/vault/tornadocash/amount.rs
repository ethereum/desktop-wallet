use alloy_primitives::{
    U256,
    utils::{format_units, parse_units},
};
use kohaku_tornadocash::{Asset, Pool};

use crate::vault::tornadocash::TornadoNote;

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
    #[error("held notes cannot make {amount} wei exactly")]
    CannotMake { amount: U256 },
    #[error("held notes cannot cover {amount} wei")]
    CannotCover { amount: U256 },
    #[error("invalid decimal amount `{0}`")]
    Parse(String),
}

/// Splits `amount` into native ETH pool deposits, largest denomination first.
///
/// # Errors
/// Returns an error when `amount` is zero, the chain has no native pools, or `amount` is not a
/// sum of those denominations.
pub fn deposit_pools(amount: U256, chain_id: u64) -> Result<Vec<Pool>, AmountError> {
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    deposit_pools_for_asset(amount, chain_id, &Asset::ETH)
}

/// Splits an amount in base units using only this chain and asset.
pub fn deposit_pools_for_asset(
    amount: U256,
    chain_id: u64,
    asset: &Asset,
) -> Result<Vec<Pool>, AmountError> {
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    let pools = asset_pools(chain_id, asset);
    if pools.is_empty() {
        return Err(AmountError::NoNativePools { chain_id });
    }
    let mut remaining = amount;
    let mut chosen = Vec::new();
    for pool in &pools {
        let denom = U256::from(pool.amount_wei);
        let count = u64_from(remaining / denom);
        for _ in 0..count {
            chosen.push(pool.clone());
        }
        remaining -= denom * U256::from(count);
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
    let denoms = denominations(&available);
    let mut remaining = amount;
    let mut chosen = Vec::new();
    for denom in denoms {
        let unit = U256::from(denom);
        let want = u64_from(remaining / unit);
        let taken = take(
            &mut available,
            denom,
            usize::try_from(want).unwrap_or(usize::MAX),
        );
        remaining -= unit * U256::from(taken.len() as u64);
        chosen.extend(taken);
    }
    if !remaining.is_zero() {
        return Err(AmountError::CannotMake { amount });
    }
    Ok(chosen)
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
    let mut available = eligible(notes);
    let denoms = denominations(&available);
    let mut remaining = amount;
    let mut chosen = Vec::new();
    for denom in denoms {
        let unit = U256::from(denom);
        let want = usize::try_from(u64_from(remaining / unit)).unwrap_or(usize::MAX);
        let taken = take(&mut available, denom, want);
        remaining -= unit * U256::from(taken.len() as u64);
        chosen.extend(taken);
    }
    if remaining.is_zero() {
        return Ok(chosen);
    }
    available.sort_by_key(|note| note.pool.amount_wei);
    let Some(extra) = available
        .into_iter()
        .find(|note| U256::from(note.pool.amount_wei) >= remaining)
    else {
        return Err(AmountError::CannotCover { amount });
    };
    chosen.push(extra);
    Ok(chosen)
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

fn asset_pools(chain_id: u64, asset: &Asset) -> Vec<Pool> {
    let mut pools: Vec<Pool> = Pool::POOLS
        .iter()
        .filter(|pool| pool.chain_id == chain_id && &pool.asset == asset)
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

fn denominations(notes: &[TornadoNote]) -> Vec<u128> {
    let mut denoms: Vec<u128> = notes.iter().map(|note| note.pool.amount_wei).collect();
    denoms.sort_unstable();
    denoms.dedup();
    denoms.reverse();
    denoms
}

fn take(notes: &mut Vec<TornadoNote>, denom: u128, count: usize) -> Vec<TornadoNote> {
    let mut taken = Vec::new();
    notes.retain(|note| {
        if taken.len() < count && note.pool.amount_wei == denom {
            taken.push(note.clone());
            false
        } else {
            true
        }
    });
    taken
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
    fn a_fraction_of_the_smallest_pool_is_rejected() {
        assert!(deposit_pools(parse_eth("0.05").expect("0.05"), 1).is_err());
        assert!(deposit_pools(parse_eth("1.15").expect("1.15"), 1).is_err());
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
