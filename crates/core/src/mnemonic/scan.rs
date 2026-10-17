use std::future::Future;

use alloy_primitives::Address;

use super::{Mnemonic, MnemonicError};
use crate::network::NetworkEndpoint;

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

impl Mnemonic {
    /// Derives standard EOAs for `profile_index` and checks them on `endpoint` in batches of
    /// 10, until a fully unused batch or [`MAX_INDEX`].
    pub async fn scan_standard_eoas(
        &self,
        profile_index: u32,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<EoaScan, MnemonicError> {
        self.scan_with(profile_index, |address| async move {
            Ok(endpoint.has_activity(address).await?)
        })
        .await
    }

    async fn scan_with<F, Fut>(
        &self,
        profile_index: u32,
        mut is_used: F,
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
                let address = self.standard_address(address_index, profile_index)?;
                scanned.push((address_index, address));
                if is_used(address).await? {
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

    use super::*;

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
        let scan = mnemonic
            .scan_with(0, {
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

    #[tokio::test]
    async fn unused_first_batch_yields_empty_addresses() {
        let (scan, inspected) = scan_used(&[]).await;
        assert_eq!(inspected, 10);
        assert!(scan.addresses.is_empty());
        assert_eq!(scan.last_used, None);
        assert_eq!(scan.next_unused, 0);
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
}
