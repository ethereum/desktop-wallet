use alloy_transport::TransportError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum NetworkEndpointConfig {
    HttpProvider { url: String },
}

#[async_trait]
pub trait NetworkEndpoint {
    async fn network_id(&self) -> Result<u64, TransportError>;
    async fn block_height(&self) -> Result<u64, TransportError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkEndpointError {
    #[error("rpc error: {0}")]
    Rpc(#[from] alloy_transport::TransportError),
    #[error("endpoint serves chain {found}, but the network is configured as chain {expected}")]
    ChainMismatch { expected: u64, found: u64 },
}
