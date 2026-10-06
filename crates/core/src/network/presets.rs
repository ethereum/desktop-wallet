use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

use crate::network::{Network, NetworkId};

/// A network known by name. Endpoints are always the user's own.
///
/// An instance opens for any [`NetworkId`], with or without a preset.
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
    pub fn network(self) -> Network {
        let native_asset = match self {
            Self::Mainnet => "eth",
            Self::Sepolia => "sepEth",
            Self::Local => "devEth",
        };
        Network {
            id: self.network_id(),
            native_asset: native_asset.to_string(),
        }
    }
}
