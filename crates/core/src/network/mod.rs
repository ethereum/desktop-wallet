pub use alloy::SimpleNetworkEndpoint;
pub use endpoint::NetworkEndpoint;
pub use logs::logs_in_range;
pub use presets::SupportedNetwork;
use serde::{Deserialize, Serialize};

pub mod alloy;
pub mod db;
pub mod endpoint;
pub mod logs;
pub mod presets;

pub const DEFAULT_EVENT_BLOCK_RANGE: u64 = 500;
pub const DEFAULT_LOCAL_NODE_PORT: u16 = 8545;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub network_id: NetworkId,
    pub name: String,
    pub native_asset: String,
    pub config: NetworkConfigKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkConfigKind {
    SimpleProvider(SimpleProviderConfig),
    LocalNode(LocalNodeConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleProviderConfig {
    pub url: String,
    pub event_block_range: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNodeConfig {
    pub port: u16,
    pub event_block_range: u64,
}

impl NetworkConfigKind {
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::SimpleProvider(_) => "simple-provider",
            Self::LocalNode(_) => "local-node",
        }
    }
}

impl NetworkConfig {
    #[must_use]
    pub fn http_rpc_url(&self) -> String {
        match &self.config {
            NetworkConfigKind::SimpleProvider(config) => config.url.clone(),
            NetworkConfigKind::LocalNode(config) => {
                format!("http://127.0.0.1:{}", config.port)
            }
        }
    }
}
