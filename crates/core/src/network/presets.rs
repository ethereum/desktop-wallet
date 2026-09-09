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
