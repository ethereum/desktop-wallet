use serde::{Deserialize, Serialize};

use crate::network::endpoint::NetworkEndpointConfig;

pub mod alloy;
pub mod db;
pub mod endpoint;
pub mod presets;

pub use alloy::SimpleNetworkEndpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub network_id: NetworkId,
    pub name: String,
    pub native_token: String,
    pub endpoints: Vec<NetworkEndpointConfig>,
}
