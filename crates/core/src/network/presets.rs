use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

use crate::network::{
    DEFAULT_EVENT_BLOCK_RANGE, DEFAULT_LOCAL_NODE_PORT, LocalNodeConfig, NetworkConfig,
    NetworkConfigKind, NetworkId, SimpleProviderConfig,
};

/// A network with known defaults for a new instance.
///
/// Presets only seed defaults. An instance opens for any [`NetworkId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, EnumString)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum NetworkPreset {
    Mainnet,
    Sepolia,
    Local,
}

impl NetworkPreset {
    #[must_use]
    pub fn from_network_id(network_id: NetworkId) -> Option<Self> {
        Self::iter().find(|preset| preset.network_id() == network_id)
    }

    #[must_use]
    pub const fn network_id(self) -> NetworkId {
        match self {
            Self::Mainnet => NetworkId(1),
            Self::Sepolia => NetworkId(11_155_111),
            Self::Local => NetworkId(31_337),
        }
    }

    #[must_use]
    pub const fn native_asset(self) -> &'static str {
        match self {
            Self::Mainnet => "eth",
            Self::Sepolia => "sepEth",
            Self::Local => "devEth",
        }
    }

    // Placeholder public RPCs for the walking skeleton. Shipping every new
    // instance's traffic to one host is a principle-3 risk (the provider can
    // correlate addresses). Replace before any real use.
    #[must_use]
    pub const fn rpc_url(self) -> &'static str {
        match self {
            Self::Mainnet => "https://ethereum.publicnode.com",
            Self::Sepolia => "https://ethereum-sepolia-rpc.publicnode.com",
            Self::Local => "http://127.0.0.1:8545",
        }
    }

    #[must_use]
    pub fn default_config(self) -> NetworkConfig {
        let config = match self {
            Self::Local => NetworkConfigKind::LocalNode(LocalNodeConfig {
                port: DEFAULT_LOCAL_NODE_PORT,
                event_block_range: DEFAULT_EVENT_BLOCK_RANGE,
            }),
            Self::Mainnet | Self::Sepolia => {
                NetworkConfigKind::SimpleProvider(SimpleProviderConfig {
                    url: self.rpc_url().to_string(),
                    event_block_range: DEFAULT_EVENT_BLOCK_RANGE,
                })
            }
        };
        NetworkConfig {
            network_id: self.network_id(),
            name: "default".to_string(),
            native_asset: self.native_asset().to_string(),
            config,
        }
    }
}
