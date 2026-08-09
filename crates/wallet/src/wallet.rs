use std::sync::Arc;

use alloy::primitives::U256;
use ethereum_desktop_wallet_core::{
    asset::AssetId,
    executor::{Executor, ExecutorError},
    vault::{Vault, VaultError},
};
use futures::future::try_join_all;

pub struct Wallet {
    pub executor: Arc<dyn Executor>,
    pub vaults: Vec<Arc<dyn Vault>>,
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("executor error: {0}")]
    Executor(#[from] ExecutorError),
    #[error("vault error: {0}")]
    Vault(#[from] VaultError),
}

impl Wallet {
    pub fn new(executor: Arc<dyn Executor>, vaults: Vec<Arc<dyn Vault>>) -> Self {
        Self { executor, vaults }
    }

    /// Returns the total balance of the given asset across all vaults.
    ///
    /// # Errors
    /// Returns an error if any of the vaults fail to return a balance.
    pub async fn balance(&self, asset: AssetId) -> Result<U256, WalletError> {
        let balances = try_join_all(self.vaults.iter().map(|v| v.balance(&asset))).await?;
        let balance = balances.into_iter().fold(U256::ZERO, |a, b| a + b);
        Ok(balance)
    }
}
