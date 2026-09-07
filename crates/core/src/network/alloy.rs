use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_transport::TransportError;
use async_trait::async_trait;
use reqwest::Url;

use super::endpoint::NetworkEndpoint;

#[derive(Debug, Clone)]
pub struct SimpleNetworkEndpoint {
    pub provider: DynProvider,
}

#[async_trait]
impl NetworkEndpoint for SimpleNetworkEndpoint {
    async fn network_id(&self) -> Result<u64, TransportError> {
        self.provider.get_chain_id().await
    }
    async fn block_height(&self) -> Result<u64, TransportError> {
        self.provider.get_block_number().await
    }
}

impl SimpleNetworkEndpoint {
    #[must_use]
    pub fn new(provider: DynProvider) -> Self {
        Self { provider }
    }

    #[must_use]
    pub fn new_http(url: Url) -> Self {
        Self::from(ProviderBuilder::new().connect_http(url).erased())
    }
}

impl From<DynProvider> for SimpleNetworkEndpoint {
    fn from(provider: DynProvider) -> Self {
        Self::new(provider)
    }
}
