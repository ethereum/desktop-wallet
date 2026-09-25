use alloy_consensus::TxEnvelope;
use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_rpc_types_eth::{Filter, Log, TransactionReceipt, TransactionRequest};
use async_trait::async_trait;
use reqwest::Url;

use super::endpoint::{FeeEstimate, NetworkEndpoint, NetworkEndpointError};

#[derive(Debug, Clone)]
pub struct SimpleNetworkEndpoint {
    provider: DynProvider,
}

#[async_trait]
impl NetworkEndpoint for SimpleNetworkEndpoint {
    async fn chain_id(&self) -> Result<u64, NetworkEndpointError> {
        self.provider.get_chain_id().await.map_err(backend)
    }

    async fn block_height(&self) -> Result<u64, NetworkEndpointError> {
        self.provider.get_block_number().await.map_err(backend)
    }

    async fn balance(&self, address: Address) -> Result<U256, NetworkEndpointError> {
        self.provider.get_balance(address).await.map_err(backend)
    }

    async fn code_at(&self, address: Address) -> Result<Bytes, NetworkEndpointError> {
        self.provider.get_code_at(address).await.map_err(backend)
    }

    async fn transaction_count(&self, address: Address) -> Result<u64, NetworkEndpointError> {
        self.provider
            .get_transaction_count(address)
            .await
            .map_err(backend)
    }

    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, NetworkEndpointError> {
        self.provider.get_logs(filter).await.map_err(backend)
    }

    async fn call(&self, tx: TransactionRequest) -> Result<Bytes, NetworkEndpointError> {
        self.provider.call(tx).await.map_err(backend)
    }

    async fn estimate_gas(&self, tx: TransactionRequest) -> Result<u64, NetworkEndpointError> {
        self.provider.estimate_gas(tx).await.map_err(backend)
    }

    async fn estimate_fees(&self) -> Result<FeeEstimate, NetworkEndpointError> {
        let fees = self
            .provider
            .estimate_eip1559_fees()
            .await
            .map_err(backend)?;
        Ok(FeeEstimate {
            max_fee_per_gas: fees.max_fee_per_gas,
            max_priority_fee_per_gas: fees.max_priority_fee_per_gas,
        })
    }

    async fn send_transaction(&self, tx: TxEnvelope) -> Result<TxHash, NetworkEndpointError> {
        let pending = self.provider.send_tx_envelope(tx).await.map_err(backend)?;
        Ok(*pending.tx_hash())
    }

    async fn receipt(
        &self,
        tx: TxHash,
    ) -> Result<Option<TransactionReceipt>, NetworkEndpointError> {
        self.provider
            .get_transaction_receipt(tx)
            .await
            .map_err(backend)
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

fn backend(error: impl std::error::Error + Send + Sync + 'static) -> NetworkEndpointError {
    NetworkEndpointError::Backend(Box::new(error))
}
