use std::future::Future;

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;

use super::{Mnemonic, MnemonicError};
use crate::network::SimpleNetworkEndpoint;

const BATCH_SIZE: u32 = 10;
const MAX_INDEX: u32 = 1000;

/// Result of scanning a profile's standard EOAs for on-chain use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EoaScan {
    /// Addresses from index 0 through the last used index, inclusive.
    /// Empty when nothing was used.
    pub addresses: Vec<(u32, Address)>,
    pub last_used: Option<u32>,
    pub next_unused: u32,
}

/// On-chain signals used to decide whether an EOA has been used.
pub struct EoaActivity {
    pub nonce: u64,
    pub code: Bytes,
    pub balance: U256,
}

impl EoaActivity {
    /// An EOA is used when it has a non-zero nonce, code, or native ETH balance.
    ///
    /// TODO: also treat the address as used if it has received any ERC-20, so a
    /// dust-token airdrop with zero ETH / nonce / code would not look unused.
    /// Exhaustive detection is unsolved on a plain JSON-RPC node: there is no
    /// `eth_getAllTokens(address)`. Practical options all have holes —
    /// `balanceOf` against a token list (misses unknown tokens), `eth_getLogs`
    /// for `Transfer(to=address)` from genesis (needs a log-indexed node, is
    /// range-limited, and still misses non-standard tokens that do not emit
    /// `Transfer`), or a third-party indexer (same completeness problem, plus a
    /// privacy/correlation leak). How to do this exhaustively?
    #[must_use]
    pub fn is_used(&self) -> bool {
        self.nonce != 0 || !self.code.is_empty() || !self.balance.is_zero()
    }
}

/// Inspects `address` on `provider`. Short-circuits on the first used signal.
pub async fn inspect_eoa(
    provider: &SimpleNetworkEndpoint,
    address: Address,
) -> Result<bool, MnemonicError> {
    let nonce = provider.provider.get_transaction_count(address).await?;
    if nonce != 0 {
        return Ok(true);
    }

    let code = provider.provider.get_code_at(address).await?;
    if !code.is_empty() {
        return Ok(true);
    }

    let balance = provider.provider.get_balance(address).await?;
    Ok(EoaActivity {
        nonce,
        code,
        balance,
    }
    .is_used())
}

/// Derives standard EOAs for `profile_index` and scans them in batches of 10
/// until a fully unused batch, or until [`MAX_INDEX`].
pub async fn scan_standard_eoas(
    mnemonic: &Mnemonic,
    profile_index: impl Into<Option<u32>>,
    provider: &SimpleNetworkEndpoint,
) -> Result<EoaScan, MnemonicError> {
    let profile_index = profile_index.into().unwrap_or(0);
    scan_with(mnemonic, profile_index, |address| {
        let provider = provider.clone();
        async move { inspect_eoa(&provider, address).await }
    })
    .await
}

async fn scan_with<F, Fut>(
    mnemonic: &Mnemonic,
    profile_index: u32,
    mut inspect: F,
) -> Result<EoaScan, MnemonicError>
where
    F: FnMut(Address) -> Fut,
    Fut: Future<Output = Result<bool, MnemonicError>>,
{
    let mut scanned = Vec::new();
    let mut last_used = None;
    let mut start = 0;

    loop {
        if start >= MAX_INDEX {
            return Err(MnemonicError::ScanLimit(MAX_INDEX));
        }

        let mut batch_used = false;
        for address_index in start..start.saturating_add(BATCH_SIZE) {
            let address = mnemonic.standard_address(address_index, profile_index)?;
            scanned.push((address_index, address));
            if inspect(address).await? {
                last_used = Some(address_index);
                batch_used = true;
            }
        }

        if !batch_used {
            break;
        }
        start = start.saturating_add(BATCH_SIZE);
    }

    let addresses = match last_used {
        Some(last) => scanned
            .into_iter()
            .take((last as usize).saturating_add(1))
            .collect(),
        None => Vec::new(),
    };
    let next_unused = last_used.map_or(0, |index| index.saturating_add(1));

    Ok(EoaScan {
        addresses,
        last_used,
        next_unused,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
    };

    use alloy_primitives::{Bytes, U64, U256, address};
    use alloy_signer_local::PrivateKeySigner;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::test_support::mocked_provider;

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn fixture() -> Mnemonic {
        Mnemonic::parse(FIXTURE).unwrap()
    }

    async fn scan_used(used: &[u32]) -> (EoaScan, u32) {
        let mnemonic = fixture();
        let used: HashSet<Address> = used
            .iter()
            .map(|index| mnemonic.standard_address(*index, None).unwrap())
            .collect();
        let used = Arc::new(used);
        let inspected = Arc::new(AtomicU32::new(0));
        let scan = scan_with(&mnemonic, 0, {
            let used = Arc::clone(&used);
            let inspected = Arc::clone(&inspected);
            move |address| {
                let used = Arc::clone(&used);
                let inspected = Arc::clone(&inspected);
                async move {
                    inspected.fetch_add(1, Ordering::SeqCst);
                    Ok(used.contains(&address))
                }
            }
        })
        .await
        .unwrap();
        (scan, inspected.load(Ordering::SeqCst))
    }

    #[test]
    fn standard_address_matches_known_answer_key() {
        let mnemonic = fixture();
        let key = mnemonic.standard_address_key(0, None).unwrap();
        let expected = PrivateKeySigner::from_signing_key(key).address();
        assert_eq!(mnemonic.standard_address(0, None).unwrap(), expected);
        assert_eq!(
            expected,
            address!("0x9858EfFD232B4033E47d90003D41EC34EcaEda94")
        );
    }

    #[test]
    fn eoa_is_used_for_nonce_code_or_balance() {
        let unused = EoaActivity {
            nonce: 0,
            code: Bytes::new(),
            balance: U256::ZERO,
        };
        assert!(!unused.is_used());

        assert!(
            EoaActivity {
                nonce: 1,
                code: Bytes::new(),
                balance: U256::ZERO,
            }
            .is_used()
        );
        assert!(
            EoaActivity {
                nonce: 0,
                code: Bytes::from_static(&[0xef]),
                balance: U256::ZERO,
            }
            .is_used()
        );
        assert!(
            EoaActivity {
                nonce: 0,
                code: Bytes::new(),
                balance: U256::from(1),
            }
            .is_used()
        );
    }

    #[tokio::test]
    async fn unused_first_batch_yields_empty_addresses() {
        let (scan, inspected) = scan_used(&[]).await;
        assert_eq!(inspected, 10);
        assert!(scan.addresses.is_empty());
        assert_eq!(scan.last_used, None);
        assert_eq!(scan.next_unused, 0);
    }

    #[tokio::test]
    async fn used_at_five_continues_then_truncates_to_last_used() {
        let (scan, inspected) = scan_used(&[5]).await;
        assert_eq!(inspected, 20);
        assert_eq!(scan.last_used, Some(5));
        assert_eq!(scan.next_unused, 6);
        assert_eq!(
            scan.addresses
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            (0..=5).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn used_at_zero_and_fifteen_scans_three_batches() {
        let (scan, inspected) = scan_used(&[0, 15]).await;
        assert_eq!(inspected, 30);
        assert_eq!(scan.last_used, Some(15));
        assert_eq!(scan.next_unused, 16);
        assert_eq!(scan.addresses.len(), 16);
        assert_eq!(scan.addresses[0].0, 0);
        assert_eq!(scan.addresses[15].0, 15);
    }

    #[tokio::test]
    async fn inspect_eoa_short_circuits_on_nonce() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));
        let used = inspect_eoa(&mocked_provider(&asserter), Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(used);
        assert!(
            asserter.read_q().is_empty(),
            "a non-zero nonce must not fetch code or balance"
        );
    }

    #[tokio::test]
    async fn inspect_eoa_unused_queries_nonce_code_and_balance() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&Bytes::new());
        asserter.push_success(&U256::ZERO);
        let used = inspect_eoa(&mocked_provider(&asserter), Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(!used);
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn inspect_eoa_treats_code_as_used() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&Bytes::from_static(&[0xef, 0x01, 0x00]));
        let used = inspect_eoa(&mocked_provider(&asserter), Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(used);
        assert!(
            asserter.read_q().is_empty(),
            "code must not fetch balance once it is already used"
        );
    }

    #[tokio::test]
    async fn inspect_eoa_treats_balance_as_used() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&Bytes::new());
        asserter.push_success(&U256::from(1));
        let used = inspect_eoa(&mocked_provider(&asserter), Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(used);
    }
}
