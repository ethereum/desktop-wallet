//! Balances held across a profile's vaults.

use futures::future::{join_all, try_join_all};

use crate::{
    asset::AssetId,
    vault::{Balance, Vault, VaultError, VaultId, Visibility},
};

/// One vault's balance of each asset, in the order of the assets passed to [`read`].
#[derive(Debug)]
pub struct VaultBalances {
    pub tag: &'static str,
    pub vault: VaultId,
    pub visibility: Visibility,
    pub balances: Result<Vec<Balance>, VaultError>,
}

/// One asset's holdings across the vaults that could be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetTotals {
    pub public: Balance,
    pub private: Balance,
}

impl AssetTotals {
    #[must_use]
    pub fn total(&self) -> Balance {
        self.public.saturating_add(self.private)
    }
}

/// Reads every vault's balance of every asset, all concurrently.
///
/// A vault fails as a whole if any one of its assets fails to read. It keeps that error, and
/// does not stop the others being read.
pub async fn read(vaults: &[Box<dyn Vault>], assets: &[AssetId]) -> Vec<VaultBalances> {
    join_all(vaults.iter().map(|vault| async move {
        VaultBalances {
            tag: vault.tag(),
            vault: vault.id(),
            visibility: vault.visibility(),
            balances: try_join_all(assets.iter().map(|asset| vault.balance(asset))).await,
        }
    }))
    .await
}

/// The totals of each of `asset_count` assets across the vaults that were read; zero for each
/// when there are none.
///
/// Balances are matched to assets by position, so every vault must have been read with the same
/// assets in the same order.
#[must_use]
pub fn totals(vaults: &[VaultBalances], asset_count: usize) -> Vec<AssetTotals> {
    let mut totals = vec![AssetTotals::default(); asset_count];
    for vault in vaults {
        let Ok(balances) = &vault.balances else {
            continue;
        };
        for (totals, balance) in totals.iter_mut().zip(balances) {
            let side = match vault.visibility {
                Visibility::Public => &mut totals.public,
                Visibility::Private => &mut totals.private,
            };
            *side = side.saturating_add(*balance);
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};

    use super::*;

    const TOKEN: Address = address!("0x00000000000000000000000000000000000000aa");

    struct FixedVault {
        native: u64,
        token: u64,
    }

    struct FailingVault;

    #[async_trait::async_trait]
    impl Vault for FixedVault {
        fn tag(&self) -> &'static str {
            "fixed"
        }

        fn id(&self) -> VaultId {
            VaultId::Address(Address::ZERO)
        }

        fn visibility(&self) -> Visibility {
            Visibility::Public
        }

        async fn balance(&self, asset: &AssetId) -> Result<Balance, VaultError> {
            Ok(Balance::spendable(U256::from(match asset {
                AssetId::Native => self.native,
                AssetId::Erc20(_) => self.token,
            })))
        }
    }

    #[async_trait::async_trait]
    impl Vault for FailingVault {
        fn tag(&self) -> &'static str {
            "failing"
        }

        fn id(&self) -> VaultId {
            VaultId::Address(Address::ZERO)
        }

        fn visibility(&self) -> Visibility {
            Visibility::Public
        }

        async fn balance(&self, _asset: &AssetId) -> Result<Balance, VaultError> {
            Err(VaultError::UnsupportedVaultId(self.id()))
        }
    }

    fn spendable(amount: u64) -> Balance {
        Balance::spendable(U256::from(amount))
    }

    /// Both visibilities together, without [`AssetTotals::total`].
    fn held(totals: &AssetTotals) -> Balance {
        totals.public.saturating_add(totals.private)
    }

    fn read_vault(
        visibility: Visibility,
        balances: Result<Vec<Balance>, VaultError>,
    ) -> VaultBalances {
        VaultBalances {
            tag: "fixed",
            vault: VaultId::Address(Address::ZERO),
            visibility,
            balances,
        }
    }

    #[tokio::test]
    async fn read_keeps_the_order_of_the_assets() {
        let vaults: [Box<dyn Vault>; 1] = [Box::new(FixedVault {
            native: 1,
            token: 2,
        })];
        let read = read(&vaults, &[AssetId::Erc20(TOKEN), AssetId::Native]).await;
        assert_eq!(
            read[0].balances.as_ref().unwrap(),
            &[spendable(2), spendable(1)]
        );
    }

    #[tokio::test]
    async fn read_reads_the_other_vaults_when_one_fails() {
        let vaults: [Box<dyn Vault>; 2] = [
            Box::new(FailingVault),
            Box::new(FixedVault {
                native: 1,
                token: 2,
            }),
        ];
        let read = read(&vaults, &[AssetId::Native]).await;
        assert!(read[0].balances.is_err());
        assert_eq!(read[1].balances.as_ref().unwrap(), &[spendable(1)]);
    }

    #[test]
    fn totals_sum_each_asset_across_vaults() {
        let vaults = [
            read_vault(Visibility::Public, Ok(vec![spendable(1), spendable(10)])),
            read_vault(Visibility::Public, Ok(vec![spendable(2), spendable(20)])),
        ];
        let held: Vec<Balance> = totals(&vaults, 2).iter().map(held).collect();
        assert_eq!(held, [spendable(3), spendable(30)]);
    }

    #[test]
    fn totals_sum_pending_across_vaults() {
        let pending = |amount: u64| Balance {
            spendable: U256::ZERO,
            pending: U256::from(amount),
        };
        let vaults = [
            read_vault(Visibility::Public, Ok(vec![pending(1)])),
            read_vault(Visibility::Public, Ok(vec![pending(2)])),
        ];
        assert_eq!(held(&totals(&vaults, 1)[0]).pending, U256::from(3));
    }

    #[test]
    fn totals_keep_public_and_private_apart() {
        let vaults = [
            read_vault(Visibility::Public, Ok(vec![spendable(1)])),
            read_vault(Visibility::Private, Ok(vec![spendable(2)])),
        ];
        assert_eq!(
            totals(&vaults, 1),
            [AssetTotals {
                public: spendable(1),
                private: spendable(2),
            }]
        );
    }

    #[test]
    fn totals_leave_out_a_vault_that_failed() {
        let vaults = [
            read_vault(
                Visibility::Public,
                Err(VaultError::UnsupportedVaultId(VaultId::Address(
                    Address::ZERO,
                ))),
            ),
            read_vault(Visibility::Public, Ok(vec![spendable(1)])),
        ];
        assert_eq!(held(&totals(&vaults, 1)[0]), spendable(1));
    }

    #[test]
    fn the_total_includes_private_holdings() {
        let totals = AssetTotals {
            public: spendable(1),
            private: spendable(2),
        };
        assert_eq!(totals.total(), spendable(3));
    }

    #[test]
    fn no_vaults_total_zero() {
        assert_eq!(totals(&[], 2), [AssetTotals::default(); 2]);
    }
}
