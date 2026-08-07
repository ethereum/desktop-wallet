use alloy::primitives::{Address, U256};

use crate::profile::{AssetId, Call, VaultId};

pub mod simple;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub enum VaultError {
    #[error("Unsupported vault id: {0:?}")]
    UnsupportedVaultId(VaultId),
    #[error(transparent)]
    Inner(Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait::async_trait]
pub trait Vault: Send + Sync {
    fn id(&self) -> VaultId;
    async fn balance(&self, asset: &AssetId) -> Result<U256, VaultError>;

    /// Returns a list of [`Call`]s that, when executed from the `from` address, will deposit
    /// the specified `amount` of the given `asset_id` into the vault.
    async fn deposit(
        &self,
        from: Address,
        asset: &AssetId,
        amount: U256,
    ) -> Result<Vec<Call>, VaultError>;

    /// Returns a list of [`Call`]s that, when executed from any address, will withdraw the specified `amount` of
    /// the given `asset_id` to the given `to` location.
    async fn withdraw(
        &self,
        to: &VaultId,
        asset: &AssetId,
        amount: U256,
    ) -> Result<Vec<Call>, VaultError>;
}
