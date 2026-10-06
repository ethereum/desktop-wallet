use alloy_primitives::{Address, U256};

use super::{Instance, InstanceError};
use crate::{
    account::AccountRecord,
    asset::{Asset, AssetError, AssetId},
    network::{NetworkEndpoint, db::NetworkDb},
    profile::{ProfileError, ProfileRecord},
};

/// What one account holds of the assets enabled for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBalances {
    pub account: AccountRecord,
    pub address: Address,
    /// The native asset first, then the enabled assets.
    pub balances: Vec<(AssetId, U256)>,
}

impl Instance {
    pub async fn assets(&self) -> Result<Vec<Asset>, InstanceError> {
        Ok(self.network_db().get_assets().await?)
    }

    /// The configured asset `selector` names: its symbol, case-insensitively, or its contract
    /// address.
    pub async fn asset(&self, selector: &str) -> Result<Asset, InstanceError> {
        if selector.eq_ignore_ascii_case(&self.network.native_asset) {
            return Err(InstanceError::Asset(AssetError::Native));
        }
        let address = selector.parse::<Address>().ok();
        let matches: Vec<Asset> = self
            .assets()
            .await?
            .into_iter()
            .filter(|asset| {
                asset.symbol.eq_ignore_ascii_case(selector)
                    || match asset.id {
                        AssetId::Erc20(contract)
                        | AssetId::Erc1155 {
                            address: contract, ..
                        } => Some(contract) == address,
                        AssetId::Native => false,
                    }
            })
            .collect();
        match <[Asset; 1]>::try_from(matches) {
            Ok([asset]) => Ok(asset),
            Err(matches) if matches.is_empty() => {
                Err(InstanceError::UnknownAsset(selector.to_string()))
            }
            Err(_) => Err(InstanceError::AmbiguousAsset(selector.to_string())),
        }
    }

    /// Reads `id`'s metadata through `endpoint` and adds it to the network's assets.
    pub async fn add_asset(
        &self,
        id: AssetId,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<Asset, InstanceError> {
        let mut assets = self.assets().await?;
        if assets.iter().any(|asset| asset.id == id) {
            return Err(InstanceError::DuplicateAsset(id));
        }
        let asset = Asset::fetch(id, endpoint).await?;
        assets.push(asset.clone());
        self.network_db().put_assets(&assets).await?;
        Ok(asset)
    }

    /// Assets enabled for every account of `profile`.
    pub async fn profile_assets(
        &self,
        profile: &ProfileRecord,
    ) -> Result<Vec<AssetId>, InstanceError> {
        Ok(self.profile_index().assets(profile).await?)
    }

    /// Enables `id` for every account of `profile`, or for the one account `account_id`.
    pub async fn enable_asset(
        &self,
        profile: &ProfileRecord,
        account_id: Option<u32>,
        id: AssetId,
    ) -> Result<(), InstanceError> {
        self.ensure_configured(id).await?;
        self.edit_enabled(profile, account_id, |assets| {
            if !assets.contains(&id) {
                assets.push(id);
            }
        })
        .await
    }

    /// Disables `id` for `profile`, or for the one account `account_id`. Disabling it on the
    /// profile leaves accounts that enable it themselves untouched.
    pub async fn disable_asset(
        &self,
        profile: &ProfileRecord,
        account_id: Option<u32>,
        id: AssetId,
    ) -> Result<(), InstanceError> {
        self.ensure_configured(id).await?;
        self.edit_enabled(profile, account_id, |assets| {
            assets.retain(|asset| *asset != id);
        })
        .await
    }

    /// Balances of `profile`'s address accounts: the native asset, plus the assets enabled for
    /// the account or its profile. Nothing else is queried.
    pub async fn balances(
        &self,
        profile: &ProfileRecord,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<Vec<AccountBalances>, InstanceError> {
        let profiles = self.profile_index();
        let profile_assets = profiles.assets(profile).await?;
        let mut overview = Vec::new();
        for account in profiles.accounts(profile).await? {
            let Some(address) = account.address() else {
                continue;
            };
            let mut enabled = vec![AssetId::Native];
            for id in profile_assets.iter().chain(&account.assets) {
                if !enabled.contains(id) {
                    enabled.push(*id);
                }
            }
            let mut balances = Vec::with_capacity(enabled.len());
            for id in enabled {
                balances.push((id, id.balance_of(address, endpoint).await?));
            }
            overview.push(AccountBalances {
                account,
                address,
                balances,
            });
        }
        Ok(overview)
    }

    async fn ensure_configured(&self, id: AssetId) -> Result<(), InstanceError> {
        if id == AssetId::Native {
            return Err(InstanceError::Asset(AssetError::Native));
        }
        if self.assets().await?.iter().any(|asset| asset.id == id) {
            Ok(())
        } else {
            Err(InstanceError::UnknownAsset(id.to_string()))
        }
    }

    async fn edit_enabled(
        &self,
        profile: &ProfileRecord,
        account_id: Option<u32>,
        edit: impl FnOnce(&mut Vec<AssetId>),
    ) -> Result<(), InstanceError> {
        let profiles = self.profile_index();
        match account_id {
            None => {
                let mut assets = profiles.assets(profile).await?;
                edit(&mut assets);
                profiles.set_assets(profile, &assets).await?;
            }
            Some(account_id) => {
                let accounts = profiles.accounts(profile).await?;
                let mut assets = accounts
                    .into_iter()
                    .find(|account| account.id == account_id)
                    .map(|account| account.assets)
                    .ok_or(ProfileError::UnknownAccount(account_id))?;
                edit(&mut assets);
                profiles
                    .set_account_assets(profile, account_id, assets)
                    .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Bytes;
    use alloy_transport::mock::Asserter;
    use zeroize::Zeroizing;

    use super::*;
    use crate::{network::NetworkId, test_support::mocked_provider};

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const USDC: AssetId = AssetId::Erc20(Address::repeat_byte(0xaa));

    async fn instance_with_usdc() -> (Instance, ProfileRecord) {
        let instance = Instance::in_memory(NetworkId(1337));
        let (_, profile) = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();
        instance
            .network_db()
            .put_assets(&[Asset {
                id: USDC,
                symbol: "USDC".into(),
                name: None,
                decimals: 6,
            }])
            .await
            .unwrap();
        (instance, profile)
    }

    #[tokio::test]
    async fn balances_query_only_the_native_and_enabled_assets() {
        let (instance, profile) = instance_with_usdc().await;
        instance
            .enable_asset(&profile, Some(0), USDC)
            .await
            .unwrap();

        let asserter = Asserter::new();
        asserter.push_success(&U256::from(7));
        asserter.push_success(&Bytes::from(U256::from(5).to_be_bytes::<32>().to_vec()));
        let overview = instance
            .balances(&profile, mocked_provider(&asserter).as_ref())
            .await
            .unwrap();

        assert_eq!(
            overview.len(),
            1,
            "the stealth account has no single address"
        );
        assert_eq!(
            overview[0].balances,
            vec![(AssetId::Native, U256::from(7)), (USDC, U256::from(5))]
        );
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn only_configured_non_native_assets_can_be_enabled() {
        let (instance, profile) = instance_with_usdc().await;

        assert!(matches!(
            instance.enable_asset(&profile, None, AssetId::Native).await,
            Err(InstanceError::Asset(AssetError::Native))
        ));
        assert!(matches!(
            instance
                .enable_asset(&profile, None, AssetId::Erc20(Address::repeat_byte(0xbb)))
                .await,
            Err(InstanceError::UnknownAsset(_))
        ));
        assert!(matches!(
            instance.enable_asset(&profile, Some(9), USDC).await,
            Err(InstanceError::Profile(ProfileError::UnknownAccount(9)))
        ));
    }
}
