use alloy_primitives::{U256, utils::format_units};
use kohaku_tornadocash::{Asset, Pool};

use crate::{
    asset::{NATIVE_DECIMALS, ParseAmountError, parse_amount},
    vault::tornadocash::TornadoNote,
};

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
    #[error(transparent)]
    Amount(#[from] ParseAmountError),
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
    let pools = native_pools(chain_id);
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
    if amount.is_zero() {
        return Err(AmountError::Zero);
    }
    let mut available = eligible(notes);
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
    Ok(parse_amount(amount, NATIVE_DECIMALS)?)
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

fn native_pools(chain_id: u64) -> Vec<Pool> {
    let mut pools: Vec<Pool> = Pool::POOLS
        .iter()
        .filter(|pool| pool.canonical && pool.chain_id == chain_id && is_native_eth(pool))
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
