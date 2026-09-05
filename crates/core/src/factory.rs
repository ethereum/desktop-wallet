use std::{pin::Pin, sync::Arc};

use alloy_provider::Provider;

use crate::{database::Database, executor::Executor, signer::Signer, vault::Vault};

pub struct Factory<T: ?Sized> {
    pub tag: &'static str,
    pub create: FactoryCreateFn<T>,
}

pub type FactoryCreateFn<T> =
    fn(BuildContext) -> Pin<Box<dyn Future<Output = Result<Box<T>, FactoryError>> + Send>>;

#[derive(Clone)]
pub struct BuildContext {
    pub provider: Arc<dyn Provider>,
    pub db: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    #[error("Factory `{0}` not found")]
    NotFound(String),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl<T: ?Sized> Factory<T> {
    pub const fn new(tag: &'static str, create: FactoryCreateFn<T>) -> Self {
        Self { tag, create }
    }
}

impl BuildContext {
    pub fn new(provider: Arc<dyn Provider>, db: Arc<dyn Database>) -> Self {
        Self { provider, db }
    }
}

pub async fn try_build_vault(
    tag: &str,
    build_ctx: BuildContext,
) -> Result<Box<dyn Vault>, FactoryError> {
    for factory in inventory::iter::<Factory<dyn Vault>> {
        if factory.tag == tag {
            return (factory.create)(build_ctx).await;
        }
    }

    Err(FactoryError::NotFound(tag.to_string()))
}

pub async fn try_build_executor(
    tag: &str,
    build_ctx: BuildContext,
) -> Result<Box<dyn Executor>, FactoryError> {
    for factory in inventory::iter::<Factory<dyn Executor>> {
        if factory.tag == tag {
            return (factory.create)(build_ctx).await;
        }
    }

    Err(FactoryError::NotFound(tag.to_string()))
}

pub async fn try_build_signer(
    tag: &str,
    build_ctx: BuildContext,
) -> Result<Box<dyn Signer>, FactoryError> {
    for factory in inventory::iter::<Factory<dyn Signer>> {
        if factory.tag == tag {
            return (factory.create)(build_ctx).await;
        }
    }

    Err(FactoryError::NotFound(tag.to_string()))
}

inventory::collect!(Factory<dyn Executor>);
inventory::collect!(Factory<dyn Signer>);
inventory::collect!(Factory<dyn Vault>);
