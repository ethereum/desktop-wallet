use std::{fmt, num::NonZeroU64, sync::Arc};

use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::{
    SimpleNetworkEndpoint,
    endpoint::{NetworkEndpoint, NetworkEndpointError},
};

/// A named, stored way to reach a network. Resolves into a [`NetworkEndpoint`] at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkEndpointConfig {
    pub name: String,
    pub kind: NetworkEndpointKind,
    /// The widest block span a single log request may ask this endpoint for.
    pub event_block_range: NonZeroU64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkEndpointKind {
    /// JSON-RPC over http or https, through alloy.
    Http { url: String },
}

impl NetworkEndpointConfig {
    /// An http or https endpoint. Rejects any other URL.
    pub fn http(
        name: String,
        url: &str,
        event_block_range: NonZeroU64,
    ) -> Result<Self, NetworkEndpointError> {
        let url = url.trim();
        Self::parse_http_url(url)?;
        Ok(Self {
            name,
            kind: NetworkEndpointKind::Http {
                url: url.to_string(),
            },
            event_block_range,
        })
    }

    /// Builds the runtime endpoint without contacting it.
    pub fn connect(&self) -> Result<Arc<dyn NetworkEndpoint>, NetworkEndpointError> {
        match &self.kind {
            NetworkEndpointKind::Http { url } => Ok(Arc::new(SimpleNetworkEndpoint::new_http(
                Self::parse_http_url(url)?,
            ))),
        }
    }

    fn parse_http_url(url: &str) -> Result<Url, NetworkEndpointError> {
        url.parse::<Url>()
            .ok()
            .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
            .ok_or_else(|| NetworkEndpointError::InvalidUrl(url.to_string()))
    }
}

impl fmt::Display for NetworkEndpointKind {
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
    fn only_http_and_https_urls_make_an_http_endpoint() {
        for url in ["http://127.0.0.1:8545", " https://rpc.example "] {
            assert!(
                NetworkEndpointConfig::http("a".into(), url, DEFAULT_EVENT_BLOCK_RANGE).is_ok()
            );
        }
        for url in ["ftp://rpc.example", "127.0.0.1:8545", ""] {
            assert!(matches!(
                NetworkEndpointConfig::http("a".into(), url, DEFAULT_EVENT_BLOCK_RANGE),
                Err(NetworkEndpointError::InvalidUrl(_))
            ));
        }
    }
}
