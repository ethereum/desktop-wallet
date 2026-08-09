use std::sync::Arc;

use alloy_primitives::U256;
use ethereum_desktop_wallet_core::{
    asset::AssetId,
    executor::Executor,
    profile::{Profile, ProfileError},
    vault::Vault,
};
use futures::future::try_join_all;

pub struct SimpleProfile {
    pub default_executor: Arc<dyn Executor>,
    pub executors: Vec<Arc<dyn Executor>>,
    pub vaults: Vec<Arc<dyn Vault>>,
}

impl SimpleProfile {
    pub fn new(executor: Arc<dyn Executor>, vaults: Vec<Arc<dyn Vault>>) -> Self {
        Self {
            default_executor: executor,
            executors: vec![],
            vaults,
        }
    }
}

#[async_trait::async_trait]
impl Profile for SimpleProfile {
    fn add_executor(&mut self, executor: impl Executor + 'static) {
        self.executors.push(Arc::new(executor));
    }

    fn add_vault(&mut self, vault: impl Vault + 'static) {
        self.vaults.push(Arc::new(vault));
    }

    async fn balance(&self, asset: AssetId) -> Result<U256, ProfileError> {
        let balances = try_join_all(self.vaults.iter().map(|v| v.balance(&asset))).await?;
        let balance = balances.into_iter().fold(U256::ZERO, |a, b| a + b);
        Ok(balance)
    }
}
