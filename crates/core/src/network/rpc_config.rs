use std::{fmt, num::NonZeroU64, sync::Arc, time::Duration};

use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::{
    NetworkId, NetworkPreset, SimpleNetworkEndpoint,
    endpoint::{NetworkEndpoint, NetworkEndpointError},
};

/// Name of the public RPC a new mainnet or Sepolia wallet starts on.
pub const PUBLIC_RPC_NAME: &str = "publicnode";

/// Pause between calls on the seeded public RPC. A config added later waits nothing.
pub const PUBLIC_RPC_CALL_INTERVAL: Duration = Duration::from_millis(500);

/// A named, stored way to reach a network. Resolves into a [`NetworkEndpoint`] at runtime.
///
/// One config can grow past a single URL: http today, and later ipc, light clients, or several
/// URLs behind one name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcConfig {
    pub name: String,
    pub kind: RpcConfigKind,
    /// The widest block span a single log request may ask this RPC for.
    pub event_block_range: NonZeroU64,
    /// How long to wait after one call to this RPC before the next, in milliseconds.
    pub call_interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcConfigKind {
    /// JSON-RPC over http or https, through alloy.
    Http { url: String },
}

impl RpcConfig {
    /// An http or https RPC. Rejects any other URL.
    pub fn http(
        name: String,
        url: &str,
        event_block_range: NonZeroU64,
    ) -> Result<Self, NetworkEndpointError> {
        let url = url.trim();
        Self::parse_http_url(url)?;
        Ok(Self {
            name,
            kind: RpcConfigKind::Http {
                url: url.to_string(),
            },
            event_block_range,
            call_interval_ms: 0,
        })
    }

    /// `http`, waiting `interval` between calls.
    ///
    /// # Errors
    /// Returns an error when `url` is not http or https.
    pub fn with_call_interval(
        name: String,
        url: &str,
        event_block_range: NonZeroU64,
        interval: Duration,
    ) -> Result<Self, NetworkEndpointError> {
        let mut config = Self::http(name, url, event_block_range)?;
        config.call_interval_ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
        Ok(config)
    }

    /// The public RPC for `network_id`, when this wallet ships one.
    #[must_use]
    pub fn public_default(network_id: NetworkId) -> Option<Self> {
        let url = NetworkPreset::from_network_id(network_id)?.default_rpc_url()?;
        Self::with_call_interval(
            PUBLIC_RPC_NAME.to_string(),
            url,
            super::DEFAULT_EVENT_BLOCK_RANGE,
            PUBLIC_RPC_CALL_INTERVAL,
        )
        .ok()
    }

    /// Builds the runtime connection without contacting it.
    pub fn connect(&self) -> Result<Arc<dyn NetworkEndpoint>, NetworkEndpointError> {
        match &self.kind {
            RpcConfigKind::Http { url } => Ok(Arc::new(SimpleNetworkEndpoint::new_http(
                Self::parse_http_url(url)?,
                self.event_block_range,
                Duration::from_millis(self.call_interval_ms),
            )?)),
        }
    }

    fn parse_http_url(url: &str) -> Result<Url, NetworkEndpointError> {
        super::http::parse_http_url(url)
    }
}

impl fmt::Display for RpcConfigKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { url } => write!(f, "http {url}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::DEFAULT_EVENT_BLOCK_RANGE;

    #[test]
    fn only_http_and_https_urls_make_an_http_rpc_config() {
        for url in ["http://127.0.0.1:8545", " https://rpc.example "] {
            assert!(RpcConfig::http("a".into(), url, DEFAULT_EVENT_BLOCK_RANGE).is_ok());
        }
        for url in ["ftp://rpc.example", "127.0.0.1:8545", ""] {
            assert!(matches!(
                RpcConfig::http("a".into(), url, DEFAULT_EVENT_BLOCK_RANGE),
                Err(NetworkEndpointError::InvalidUrl(_))
            ));
        }
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn mainnet_and_sepolia_start_on_publicnode() {
        let mainnet = RpcConfig::public_default(NetworkId(1)).expect("mainnet default");
        let sepolia = RpcConfig::public_default(NetworkId(11_155_111)).expect("sepolia default");
        assert!(RpcConfig::public_default(NetworkId(31_337)).is_none());
        assert_eq!(mainnet.name, PUBLIC_RPC_NAME);
        assert_eq!(
            mainnet.kind,
            RpcConfigKind::Http {
                url: "https://ethereum.publicnode.com".to_string(),
            }
        );
        assert_eq!(
            sepolia.kind,
            RpcConfigKind::Http {
                url: "https://ethereum-sepolia-rpc.publicnode.com".to_string(),
            }
        );
        assert_eq!(mainnet.event_block_range.get(), 500);
        assert_eq!(mainnet.call_interval_ms, 500);
        assert_eq!(
            RpcConfig::http(
                "other".into(),
                "https://rpc.example",
                DEFAULT_EVENT_BLOCK_RANGE
            )
            .expect("http config")
            .call_interval_ms,
            0
        );
    }
}
