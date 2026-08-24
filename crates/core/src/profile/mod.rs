use std::future::Future;

use alloy_primitives::U256;

use crate::{
    asset::AssetId,
    factory::BuildContext,
    vault::{Vault, VaultError},
};

pub mod simple;

/// A trait representing a wallet with a default executor and multiple vaults.
///
/// `add_vault` takes a constructor so the [`Profile`] can resolve the
/// [`BuildContext`] before construction starts, making any adjustments to the
/// context as needed.
#[async_trait::async_trait]
pub trait Profile {
    async fn add_vault<V, E, F, Fut>(&mut self, ctor: F) -> Result<(), ProfileError>
    where
        V: Vault + 'static,
        E: Into<VaultError>,
        F: FnOnce(BuildContext) -> Fut + Send,
        Fut: Future<Output = Result<V, E>> + Send;

    async fn balance(&self, asset: AssetId) -> Result<U256, ProfileError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("executor error: {0}")]
    Executor(#[from] crate::executor::ExecutorError),
    #[error("vault error: {0}")]
    Vault(#[from] crate::vault::VaultError),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}
