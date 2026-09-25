use alloy_consensus::TxEnvelope;
use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_rpc_types_eth::{Filter, Log, TransactionReceipt, TransactionRequest};
use async_trait::async_trait;

#[async_trait]
pub trait NetworkEndpoint: Send + Sync {
    async fn chain_id(&self) -> Result<u64, NetworkEndpointError>;
    async fn block_height(&self) -> Result<u64, NetworkEndpointError>;

    async fn balance(&self, address: Address) -> Result<U256, NetworkEndpointError>;
    /// Empty for an account with no code.
    async fn code_at(&self, address: Address) -> Result<Bytes, NetworkEndpointError>;
    /// Also `address`'s next nonce.
    async fn transaction_count(&self, address: Address) -> Result<u64, NetworkEndpointError>;

    /// One request, so a strict endpoint may refuse a wide `filter`. See
    /// [`logs_in_range`](crate::network::logs_in_range).
    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, NetworkEndpointError>;

    /// Executes `tx` against the latest block without submitting it.
    async fn call(&self, tx: TransactionRequest) -> Result<Bytes, NetworkEndpointError>;
    async fn estimate_gas(&self, tx: TransactionRequest) -> Result<u64, NetworkEndpointError>;
    async fn estimate_fees(&self) -> Result<FeeEstimate, NetworkEndpointError>;

    /// Acceptance is not inclusion: poll [`NetworkEndpoint::receipt`] for that.
    async fn send_transaction(&self, tx: TxEnvelope) -> Result<TxHash, NetworkEndpointError>;
    /// `None` while `tx` is unmined or unknown.
    async fn receipt(&self, tx: TxHash)
    -> Result<Option<TransactionReceipt>, NetworkEndpointError>;
}

/// EIP-1559 fee parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeEstimate {
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkEndpointError {
    /// The inner error type is not part of this API.
    #[error(transparent)]
    Backend(Box<dyn std::error::Error + Send + Sync>),
    #[error("endpoint serves chain {found}, but the network is configured as chain {expected}")]
    ChainMismatch { expected: u64, found: u64 },
}
