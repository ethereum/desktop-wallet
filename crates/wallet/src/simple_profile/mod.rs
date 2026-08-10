use std::sync::Arc;

use alloy_primitives::U256;
use alloy_provider::Provider;
use ethereum_desktop_wallet_core::{
    asset::AssetId,
    database::Database,
    executor::{Executor, ExecutorError},
    factory::{BuildContext, FactoryError, try_build_executor, try_build_vault},
    profile::{Profile, ProfileError},
    vault::{Vault, VaultError},
};
use futures::future::try_join_all;

use crate::{
    database::scoped::ScopedDatabaseExt,
    simple_profile::db::{SimpleProfileDatabaseError, SimpleProfileDb},
};

mod db;

pub struct SimpleProfile {
    pub default_executor: Box<dyn Executor>,
    pub executors: Vec<Box<dyn Executor>>,
    pub vaults: Vec<Box<dyn Vault>>,

    db: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleProfileError {
    #[error("database error: {0}")]
    Database(#[from] SimpleProfileDatabaseError),
    #[error("vault error: {0}")]
    Vault(#[from] VaultError),
    #[error("executor error: {0}")]
    Executor(#[from] ExecutorError),
    #[error("factory error: {0}")]
    Factory(#[from] FactoryError),
    #[error("missing default executor")]
    MissingDefaultExecutor,
}

impl SimpleProfile {
    pub fn new(
        executor: Box<dyn Executor>,
        vaults: Vec<Box<dyn Vault>>,
        db: Arc<dyn Database>,
    ) -> Self {
        Self {
            default_executor: executor,
            executors: vec![],
            vaults,
            db,
        }
    }

    pub async fn load(
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleProfileError> {
        let vaults = db.get_vaults().await?;
        let executors = db.get_executors().await?;

        let db = &db;
        let provider = &provider;
        let vaults: Vec<_> = vaults
            .into_iter()
            .map(|(tag, id)| async move {
                let scope = format!("{tag:}:{id:}");
                let db = Arc::new(db.clone().scoped(scope.as_bytes()));
                let ctx = BuildContext::new(provider.clone(), db);
                try_build_vault(&tag, ctx).await
            })
            .collect::<Vec<_>>();
        let vaults = try_join_all(vaults).await?;

        let executors: Vec<_> = executors
            .into_iter()
            .map(|(tag, id)| async move {
                let scope = format!("{tag:}:{id:}");
                let db = Arc::new(db.clone().scoped(scope.as_bytes()));
                let ctx = BuildContext::new(provider.clone(), db);
                try_build_executor(&tag, ctx).await
            })
            .collect::<Vec<_>>();
        let mut executors = try_join_all(executors).await?;

        let default_executor = if executors.len() > 0 {
            executors.remove(0)
        } else {
            return Err(SimpleProfileError::MissingDefaultExecutor);
        };

        Ok(Self {
            default_executor,
            executors,
            vaults,
            db: db.clone(),
        })
    }

    async fn save(&self) -> Result<(), SimpleProfileError> {
        let vaults: Vec<_> = self.vaults.iter().map(|v| (v.tag(), v.id())).collect();
        let executors: Vec<_> = self.executors.iter().map(|e| (e.tag(), e.id())).collect();

        self.db.put_vaults(&vaults).await?;
        self.db.put_executors(&executors).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Profile for SimpleProfile {
    async fn add_executor(
        &mut self,
        executor: impl Executor + 'static,
    ) -> Result<(), ProfileError> {
        self.executors.push(Box::new(executor));
        self.save().await?;
        Ok(())
    }

    async fn add_vault(&mut self, vault: impl Vault + 'static) -> Result<(), ProfileError> {
        self.vaults.push(Box::new(vault));
        self.save().await?;
        Ok(())
    }

    async fn balance(&self, asset: AssetId) -> Result<U256, ProfileError> {
        let balances = try_join_all(self.vaults.iter().map(|v| v.balance(&asset))).await?;
        let balance = balances.into_iter().fold(U256::ZERO, |a, b| a + b);
        Ok(balance)
    }
}

impl From<SimpleProfileError> for ProfileError {
    fn from(err: SimpleProfileError) -> Self {
        match err {
            SimpleProfileError::Vault(v) => ProfileError::Vault(v),
            SimpleProfileError::Executor(e) => ProfileError::Executor(e),
            _ => ProfileError::Other(Box::new(err)),
        }
    }
}
