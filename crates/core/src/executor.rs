use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};

use crate::call::Call;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutorId {
    Address(Address),
}

/// A unique identifier for a [`Call`] that has been sent for execution.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallId(pub B256);

/// A receipt for a [`Call`] that has been executed.
///
/// TODO: Decide whether / how we want to merge UserOperation receipts with
/// Transaction receipts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallReceipt;

/// A trait representing an address that can execute [`Call`]s.
#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    fn tag(&self) -> &'static str;
    fn id(&self) -> ExecutorId;
    fn address(&self) -> Address;

    /// Sends a list of [`Call`]s for execution. Returns a [`CallId`] that can
    /// be used to retrieve the receipt of the execution.
    ///
    /// This method will return immediately after the calls have been sent for
    /// execution, and does not wait for execution to complete or for a receipt
    /// to be available.
    async fn execute(&self, calls: &[Call]) -> Result<CallId, ExecutorError>;

    /// Retrieves the receipt of a previously sent [`Call`] using its [`CallId`].
    /// If the receipt is not yet available, this method will return `None`.
    async fn receipt(&self, id: CallId) -> Result<Option<CallReceipt>, ExecutorError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for ExecutorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorId::Address(addr) => write!(f, "addr:{addr:}"),
        }
    }
}
