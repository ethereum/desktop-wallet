use std::{fmt, num::NonZeroU64, str::FromStr};

pub use alloy::SimpleNetworkEndpoint;
pub use endpoint::NetworkEndpoint;
pub use endpoint_config::{NetworkEndpointConfig, NetworkEndpointKind};
pub use presets::NetworkPreset;
use serde::{Deserialize, Serialize};

pub mod alloy;
pub mod db;
pub mod endpoint;
pub mod endpoint_config;
mod logs;
pub mod presets;

pub const DEFAULT_EVENT_BLOCK_RANGE: NonZeroU64 = NonZeroU64::new(499).expect("non-zero");

/// Native asset of a network without a preset.
const DEFAULT_NATIVE_ASSET: &str = "eth";

/// An EVM network id. Displays and parses as a preset name when one matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkId(pub u64);

/// A chain the wallet operates on. Each network is a separate wallet instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    pub id: NetworkId,
    pub native_asset: String,
}

#[derive(Debug, thiserror::Error)]
#[error("unknown network `{0}`; expected mainnet, sepolia, local, or a numeric network id")]
pub struct ParseNetworkIdError(String);

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match NetworkPreset::from_network_id(*self) {
            Some(preset) => preset.fmt(f),
            None => self.0.fmt(f),
        }
    }
}

impl FromStr for NetworkId {
    type Err = ParseNetworkIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(preset) = s.parse::<NetworkPreset>() {
            return Ok(preset.network_id());
        }
        s.parse()
            .map(Self)
            .map_err(|_| ParseNetworkIdError(s.to_string()))
    }
}

impl Network {
    /// The preset's network, or one with a generic native asset for any other id.
    #[must_use]
    pub fn new(id: NetworkId) -> Self {
        NetworkPreset::from_network_id(id).map_or_else(
            || Self {
                id,
                native_asset: DEFAULT_NATIVE_ASSET.to_string(),
            },
            NetworkPreset::network,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse_case_insensitively_to_their_network_id() {
        assert_eq!("mainnet".parse::<NetworkId>().ok(), Some(NetworkId(1)));
        assert_eq!(
            "SEPOLIA".parse::<NetworkId>().ok(),
            Some(NetworkId(11_155_111))
        );
        assert_eq!("Local".parse::<NetworkId>().ok(), Some(NetworkId(31_337)));
    }

    #[test]
    fn any_numeric_network_id_parses() {
        assert_eq!("1337".parse::<NetworkId>().ok(), Some(NetworkId(1337)));
        assert!("anvil".parse::<NetworkId>().is_err());
    }

    #[test]
    fn display_round_trips_through_parse() {
        for id in [NetworkId(1), NetworkId(1337)] {
            assert_eq!(id.to_string().parse::<NetworkId>().ok(), Some(id));
        }
        assert_eq!(NetworkId(1).to_string(), "mainnet");
        assert_eq!(NetworkId(1337).to_string(), "1337");
    }
}
