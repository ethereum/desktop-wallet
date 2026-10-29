use serde::{Deserialize, Serialize};

use super::NetworkId;

/// Default Saga CDN. Mainnet uses it as-is. Sepolia uses `{base}/sepolia`.
pub const SAGA_CDN_BASE: &str = "https://saga.gordosoluciones.xyz";

/// The pool indexer for one network. The stored URL is the exact URL that is requested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerConfig {
    pub url: Option<String>,
    pub enabled: bool,
}

impl IndexerConfig {
    /// Mainnet and Sepolia start on Saga, turned on. Any other network starts off, with no URL.
    #[must_use]
    pub fn default_for(network_id: NetworkId) -> Self {
        match network_id.0 {
            1 => Self {
                url: Some(SAGA_CDN_BASE.to_string()),
                enabled: true,
            },
            11_155_111 => Self {
                url: Some(format!("{SAGA_CDN_BASE}/sepolia")),
                enabled: true,
            },
            _ => Self {
                url: None,
                enabled: false,
            },
        }
    }

    /// The URL to request, when the indexer is on and has one.
    #[must_use]
    pub fn effective_url(&self) -> Option<&str> {
        if self.enabled {
            self.url.as_deref()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_chain() {
        let mainnet = IndexerConfig::default_for(NetworkId(1));
        let sepolia = IndexerConfig::default_for(NetworkId(11_155_111));
        let local = IndexerConfig::default_for(NetworkId(31_337));
        assert_eq!(
            mainnet.effective_url(),
            Some("https://saga.gordosoluciones.xyz")
        );
        assert_eq!(
            sepolia.effective_url(),
            Some("https://saga.gordosoluciones.xyz/sepolia")
        );
        assert_eq!(local.effective_url(), None);
        assert!(!local.enabled);
    }
}
