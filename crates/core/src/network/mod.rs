use std::num::NonZeroU64;

pub use alloy::SimpleNetworkEndpoint;
pub use endpoint::NetworkEndpoint;
use endpoint::NetworkEndpointError;
pub use logs::logs_in_range;
pub use presets::SupportedNetwork;
use serde::{Deserialize, Serialize};

pub mod alloy;
pub mod db;
pub mod endpoint;
pub mod logs;
pub mod presets;

pub const DEFAULT_EVENT_BLOCK_RANGE: NonZeroU64 = NonZeroU64::new(499).expect("non-zero");
pub const DEFAULT_LOCAL_NODE_PORT: u16 = 8545;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub network_id: NetworkId,
    pub name: String,
    pub native_asset: String,
    pub config: NetworkConfigKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkConfigKind {
    SimpleProvider(SimpleProviderConfig),
    LocalNode(LocalNodeConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimpleProviderConfig {
    pub url: String,
    pub event_block_range: NonZeroU64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNodeConfig {
    pub port: u16,
    pub event_block_range: NonZeroU64,
}

impl NetworkConfigKind {
    // TODO: maybe replace or relocate: hand-written string mapping; strum derive.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::SimpleProvider(_) => "simple-provider",
            Self::LocalNode(_) => "local-node",
        }
    }
}

impl NetworkConfig {
    #[must_use]
    pub fn http_rpc_url(&self) -> String {
        match &self.config {
            NetworkConfigKind::SimpleProvider(config) => config.url.clone(),
            NetworkConfigKind::LocalNode(config) => {
                format!("http://127.0.0.1:{}", config.port)
            }
        }
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
