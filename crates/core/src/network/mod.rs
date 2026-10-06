use std::{fmt, num::NonZeroU64, str::FromStr};

pub use alloy::SimpleNetworkEndpoint;
pub use endpoint::NetworkEndpoint;
use endpoint::NetworkEndpointError;
pub use endpoint_config::{NetworkEndpointConfig, NetworkEndpointKind};
pub use logs::logs_in_range;
pub use presets::NetworkPreset;
use serde::{Deserialize, Serialize};

pub mod alloy;
pub mod db;
pub mod endpoint;
pub mod endpoint_config;
pub mod logs;
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

// TODO: maybe replace or relocate: first param is the endpoint; a provided method on NetworkEndpoint.
/// Rejects `endpoint` unless it serves `expected`.
///
/// Costs one round trip.
///
/// # Errors
/// [`NetworkEndpointError::NetworkMismatch`] if the endpoint serves another network.
pub async fn verify_network_id(
    endpoint: &dyn NetworkEndpoint,
    expected: NetworkId,
) -> Result<(), NetworkEndpointError> {
    let found = endpoint.network_id().await?;
    if found == expected.0 {
        return Ok(());
    }
    Err(NetworkEndpointError::NetworkMismatch {
        expected: expected.0,
        found,
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U64;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::test_support::mocked_provider;

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

    #[tokio::test]
    async fn an_endpoint_serving_the_expected_network_id_is_accepted() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));

        verify_network_id(mocked_provider(&asserter).as_ref(), NetworkId(1))
            .await
            .expect("the endpoint serves the network id it was configured as");
    }

    #[tokio::test]
    async fn an_endpoint_serving_another_network_id_is_rejected() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));

        let endpoint = mocked_provider(&asserter);

        let Err(error) = verify_network_id(endpoint.as_ref(), NetworkId(11_155_111)).await else {
            panic!("a mainnet endpoint must not pass as sepolia");
        };
        assert!(
            matches!(
                error,
                NetworkEndpointError::NetworkMismatch {
                    expected: 11_155_111,
                    found: 1,
                }
            ),
            "expected the disagreement to name both network ids, got {error}",
        );
    }
}
