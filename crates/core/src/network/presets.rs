use super::NetworkId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPreset {
    EthereumMainnet,
    LocalTestnet,
    SepoliaTestnet,
    HoodiTestnet,
}

impl NetworkPreset {
    /// Resolves a preset from a user-typed slug or network ID.
    #[must_use]
    pub fn from_input(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "1" | "ethereum" | "mainnet" => Some(Self::EthereumMainnet),
            "31337" | "local" | "testnet" => Some(Self::LocalTestnet),
            "11155111" | "sepolia" => Some(Self::SepoliaTestnet),
            "560048" | "hoodi" => Some(Self::HoodiTestnet),
            _ => None,
        }
    }

    #[must_use]
    pub const fn network_id(self) -> NetworkId {
        let id = match self {
            Self::EthereumMainnet => 1,
            Self::LocalTestnet => 31337,
            Self::SepoliaTestnet => 11_155_111,
            Self::HoodiTestnet => 560_048,
        };
        NetworkId(id)
    }

    /// The network's name, without any standing baked into it. Sepolia is named `sepolia`;
    /// that it is a testnet is [`NetworkPreset::kind`].
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "ethereum",
            Self::LocalTestnet => "local",
            Self::SepoliaTestnet => "sepolia",
            Self::HoodiTestnet => "hoodi",
        }
    }

    /// The endpoint this network is reachable at without any configuration.
    ///
    /// Only the local development node has one. Shipping a default endpoint for a public
    /// network would pick a third-party RPC provider on the user's behalf, and which provider
    /// sees their traffic is the user's call, not ours.
    #[must_use]
    pub const fn default_rpc_url(self) -> Option<&'static str> {
        match self {
            Self::LocalTestnet => Some("http://localhost:8545"),
            Self::EthereumMainnet | Self::SepoliaTestnet | Self::HoodiTestnet => None,
        }
    }

    #[must_use]
    pub const fn native_token(self) -> &'static str {
        match self {
            Self::EthereumMainnet => "ETH",
            Self::LocalTestnet => "devETH",
            Self::SepoliaTestnet => "sepETH",
            Self::HoodiTestnet => "hoodiETH",
        }
    }
}
