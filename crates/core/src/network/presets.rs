use std::{fmt, str::FromStr};

use crate::network::{NetworkConfig, NetworkId, endpoint::NetworkEndpointConfig};

// Placeholder public RPCs for the walking skeleton. Shipping every new
// instance's traffic to one host is a principle-3 risk (the provider can
// correlate addresses). Replace before any real use.
const MAINNET: NetworkPreset = NetworkPreset {
    network_id: NetworkId(1),
    name: "Mainnet",
    native_asset: "eth",
    rpc_url: "https://ethereum.publicnode.com",
};
const SEPOLIA: NetworkPreset = NetworkPreset {
    network_id: NetworkId(11_155_111),
    name: "Sepolia",
    native_asset: "sepEth",
    rpc_url: "https://ethereum-sepolia-rpc.publicnode.com",
};
const LOCAL: NetworkPreset = NetworkPreset {
    network_id: NetworkId(31_337),
    name: "Local",
    native_asset: "devEth",
    rpc_url: "http://localhost:8545",
};

pub const NETWORK_PRESETS: &[NetworkPreset] = &[MAINNET, SEPOLIA, LOCAL];

pub struct NetworkPreset {
    pub network_id: NetworkId,
    pub name: &'static str,
    pub native_asset: &'static str,
    pub rpc_url: &'static str,
}

/// Networks this binary can open. Each is a separate store and password.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SupportedNetwork {
    #[default]
    Mainnet,
    Sepolia,
    Local,
}

impl SupportedNetwork {
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Sepolia => "sepolia",
            Self::Local => "local",
        }
    }

    const fn preset(self) -> &'static NetworkPreset {
        match self {
            Self::Mainnet => &MAINNET,
            Self::Sepolia => &SEPOLIA,
            Self::Local => &LOCAL,
        }
    }

    #[must_use]
    pub fn default_config(self) -> NetworkConfig {
        let preset = self.preset();
        NetworkConfig {
            network_id: preset.network_id,
            name: "default".to_string(),
            native_asset: preset.native_asset.to_string(),
            endpoints: vec![NetworkEndpointConfig::HttpProvider {
                url: preset.rpc_url.to_string(),
            }],
        }
    }
}

impl fmt::Display for SupportedNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for SupportedNetwork {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mainnet" => Ok(Self::Mainnet),
            "sepolia" => Ok(Self::Sepolia),
            "local" => Ok(Self::Local),
            _ => Err(anyhow::anyhow!(
                "unsupported network {s:?}; expected mainnet, sepolia, or local"
            )),
        }
    }
}

impl NetworkConfig {
    #[must_use]
    pub fn presets() -> &'static [NetworkPreset] {
        NETWORK_PRESETS
    }

    pub fn from_preset(name_or_id: &str) -> Result<Self, anyhow::Error> {
        let preset = NETWORK_PRESETS
            .iter()
            .find(|p| {
                p.name.eq_ignore_ascii_case(name_or_id) || p.network_id.0.to_string() == name_or_id
            })
            .ok_or_else(|| anyhow::anyhow!("No preset found for name or id: {name_or_id}"))?;

        Ok(Self {
            network_id: preset.network_id,
            name: preset.name.to_string(),
            native_asset: preset.native_asset.to_string(),
            endpoints: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip() {
        assert_eq!(
            "mainnet".parse::<SupportedNetwork>().ok(),
            Some(SupportedNetwork::Mainnet)
        );
        assert_eq!(
            "sepolia".parse::<SupportedNetwork>().ok(),
            Some(SupportedNetwork::Sepolia)
        );
        assert_eq!(
            "local".parse::<SupportedNetwork>().ok(),
            Some(SupportedNetwork::Local)
        );
        assert_eq!(
            "Mainnet".parse::<SupportedNetwork>().ok(),
            Some(SupportedNetwork::Mainnet)
        );
        assert_eq!(
            "SEPOLIA".parse::<SupportedNetwork>().ok(),
            Some(SupportedNetwork::Sepolia)
        );
    }

    #[test]
    fn default_config_matches_the_variant() {
        assert_eq!(
            SupportedNetwork::Mainnet.default_config().network_id,
            NetworkId(1)
        );
        assert_eq!(
            SupportedNetwork::Sepolia.default_config().network_id,
            NetworkId(11_155_111)
        );
        assert_eq!(
            SupportedNetwork::Local.default_config().network_id,
            NetworkId(31_337)
        );
    }

    #[test]
    fn default_config_is_named_default_and_has_a_public_rpc() {
        let local = SupportedNetwork::Local.default_config();
        assert_eq!(local.name, "default");
        assert_eq!(local.http_rpc_url(), Some("http://localhost:8545"));
        assert_eq!(
            SupportedNetwork::Mainnet.default_config().http_rpc_url(),
            Some("https://ethereum.publicnode.com")
        );
        assert_eq!(
            SupportedNetwork::Sepolia.default_config().http_rpc_url(),
            Some("https://ethereum-sepolia-rpc.publicnode.com")
        );
    }
}
