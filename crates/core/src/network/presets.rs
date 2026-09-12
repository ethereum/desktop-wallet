use std::{fmt, str::FromStr};

use crate::network::{Network, NetworkId};

pub const NETWORK_PRESETS: &[NetworkPreset] = &[
    NetworkPreset {
        network_id: NetworkId(1),
        name: "Mainnet",
        native_asset: "eth",
    },
    NetworkPreset {
        network_id: NetworkId(11_155_111),
        name: "Sepolia",
        native_asset: "sepEth",
    },
    NetworkPreset {
        network_id: NetworkId(31_337),
        name: "Local",
        native_asset: "devEth",
    },
];

pub struct NetworkPreset {
    pub network_id: NetworkId,
    pub name: &'static str,
    pub native_asset: &'static str,
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

    #[must_use]
    pub fn preferences(self) -> Network {
        Network::from_preset(self.slug()).unwrap_or_else(|_| Network {
            network_id: NetworkId(1),
            name: "Mainnet".to_string(),
            native_asset: "eth".to_string(),
            endpoints: vec![],
        })
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
        match s {
            "mainnet" => Ok(Self::Mainnet),
            "sepolia" => Ok(Self::Sepolia),
            "local" => Ok(Self::Local),
            _ => Err(anyhow::anyhow!(
                "unsupported network {s:?}; expected mainnet, sepolia, or local"
            )),
        }
    }
}

impl Network {
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
    }
}
