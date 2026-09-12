pub use alloy::SimpleNetworkEndpoint;
pub use presets::SupportedNetwork;
use serde::{Deserialize, Serialize};

use crate::network::endpoint::NetworkEndpointConfig;

pub mod alloy;
pub mod db;
pub mod endpoint;
pub mod presets;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub network_id: NetworkId,
    pub name: String,
    pub native_asset: String,
    pub endpoints: Vec<NetworkEndpointConfig>,
}

impl Network {
    #[must_use]
    pub fn http_rpc_url(&self) -> Option<&str> {
        match self.endpoints.first() {
            Some(NetworkEndpointConfig::HttpProvider { url }) => Some(url.as_str()),
            None => None,
        }
    }
}
