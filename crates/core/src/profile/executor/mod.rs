use alloy::primitives::Address;

use crate::profile::{Call, ExecutorId};

pub mod simple;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub enum ExecutorError {
    #[error(transparent)]
    Inner(Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait::async_trait]
pub trait Executor {
    fn id(&self) -> ExecutorId;
    fn address(&self) -> Address;
    async fn send_calls(&self, calls: &[Call]) -> Result<(), ExecutorError>;
}
