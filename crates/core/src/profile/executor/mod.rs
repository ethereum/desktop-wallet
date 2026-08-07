use alloy::primitives::Address;

use crate::{call::Call, profile::ExecutorId};

pub mod simple;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub enum ExecutorError {
    #[error(transparent)]
    Inner(Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    fn id(&self) -> ExecutorId;
    fn address(&self) -> Address;

    /// Executes a list of [`Call`]s from the executor's address.
    async fn execute(&self, calls: &[Call]) -> Result<(), ExecutorError>;
}
