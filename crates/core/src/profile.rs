use alloy_primitives::U256;

use crate::{asset::AssetId, executor::Executor, vault::Vault};

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("executor error: {0}")]
    Executor(#[from] crate::executor::ExecutorError),
    #[error("vault error: {0}")]
    Vault(#[from] crate::vault::VaultError),
    #[error(transparent)]
    Inner(Box<dyn std::error::Error + Send + Sync>),
}

/// A trait representing a wallet that manages multiple vaults and executors.
#[async_trait::async_trait]
pub trait Profile {
    fn add_vault(&mut self, vault: impl Vault + 'static);
    fn add_executor(&mut self, executor: impl Executor + 'static);

    async fn balance(&self, asset: AssetId) -> Result<U256, ProfileError>;
}
