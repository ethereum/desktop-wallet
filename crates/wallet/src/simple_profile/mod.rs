use std::{future::Future, sync::Arc};

use alloy_primitives::U256;
use alloy_provider::Provider;
use edw_core::{
    asset::AssetId,
    database::Database,
    executor::{Executor, ExecutorError},
    factory::{BuildContext, FactoryError, try_build_executor, try_build_vault},
    profile::{Profile, ProfileError},
    vault::{Vault, VaultError},
};
use futures::future::try_join_all;
use uuid::Uuid;

use crate::{
    database::scoped::ScopedDatabaseExt,
    simple_profile::db::{SimpleProfileDatabaseError, SimpleProfileDb},
};

mod db;

pub struct SimpleProfile {
    pub default_executor: (Uuid, Box<dyn Executor>),
    pub vaults: Vec<(Uuid, Box<dyn Vault>)>,

    provider: Arc<dyn Provider>,
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
    pub async fn new<X, E, F, Fut>(
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
        ctor: F,
    ) -> Result<Self, SimpleProfileError>
    where
        X: Executor + 'static,
        E: Into<ExecutorError>,
        F: FnOnce(BuildContext) -> Fut + Send,
        Fut: Future<Output = Result<X, E>> + Send,
    {
        let (id, executor) = build_scoped(&provider, &db, ctor)
            .await
            .map_err(Into::into)?;

        let profile = Self {
            default_executor: (id, Box::new(executor)),
            vaults: vec![],
            provider,
            db,
        };
        profile.save().await?;
        Ok(profile)
    }

    pub async fn load(
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleProfileError> {
        let vault_entries = db.get_vaults().await?;
        let Some((executor_id, executor_tag)) = db.get_executor().await? else {
            return Err(SimpleProfileError::MissingDefaultExecutor);
        };

        let db_ref = &db;
        let provider_ref = &provider;
        let vaults: Vec<_> = vault_entries
            .into_iter()
            .map(|(id, tag)| async move {
                let db = Arc::new(db_ref.clone().scoped(id.as_bytes()));
                let ctx = BuildContext::new(provider_ref.clone(), db);
                let v = try_build_vault(&tag, ctx).await?;
                Ok::<_, SimpleProfileError>((id, v))
            })
            .collect::<Vec<_>>();
        let vaults = try_join_all(vaults).await?;

        let executor_db = Arc::new(db.clone().scoped(executor_id.as_bytes()));
        let executor_ctx = BuildContext::new(provider.clone(), executor_db);
        let executor = try_build_executor(&executor_tag, executor_ctx).await?;

        Ok(Self {
            default_executor: (executor_id, executor),
            vaults,
            provider,
            db,
        })
    }

    async fn save(&self) -> Result<(), SimpleProfileError> {
        let vaults: Vec<_> = self.vaults.iter().map(|(id, v)| (*id, v.tag())).collect();

        self.db.put_vaults(&vaults).await?;
        self.db
            .put_executor((self.default_executor.0, self.default_executor.1.tag()))
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Profile for SimpleProfile {
    async fn add_vault<V, E, F, Fut>(&mut self, ctor: F) -> Result<(), ProfileError>
    where
        V: Vault + 'static,
        E: Into<VaultError>,
        F: FnOnce(BuildContext) -> Fut + Send,
        Fut: Future<Output = Result<V, E>> + Send,
    {
        let (id, vault) = build_scoped(&self.provider, &self.db, ctor)
            .await
            .map_err(Into::into)?;

        self.vaults.push((id, Box::new(vault)));
        self.save().await?;
        Ok(())
    }

    async fn balance(&self, asset: AssetId) -> Result<U256, ProfileError> {
        let balances = try_join_all(self.vaults.iter().map(|(_, v)| v.balance(&asset))).await?;
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

/// Generates a fresh storage scope, builds a [`BuildContext`] against it, and runs
/// `ctor` against that context.
async fn build_scoped<T, E, F, Fut>(
    provider: &Arc<dyn Provider>,
    db: &Arc<dyn Database>,
    ctor: F,
) -> Result<(Uuid, T), E>
where
    F: FnOnce(BuildContext) -> Fut + Send,
    Fut: Future<Output = Result<T, E>> + Send,
{
    let id = Uuid::new_v4();
    let scoped_db = Arc::new(db.clone().scoped(id.as_bytes()));
    let ctx = BuildContext::new(provider.clone(), scoped_db);
    let value = ctor(ctx).await?;
    Ok((id, value))
}
