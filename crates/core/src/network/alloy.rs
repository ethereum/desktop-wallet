use alloy_consensus::TxEnvelope;
use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_rpc_types_eth::{Filter, Log, TransactionReceipt, TransactionRequest};
use alloy_transport::TransportError;
use async_trait::async_trait;
use reqwest::Url;

use super::endpoint::{FeeEstimate, NetworkEndpoint, NetworkEndpointError};

#[derive(Debug, Clone)]
pub struct SimpleNetworkEndpoint {
    provider: DynProvider,
}

#[async_trait]
impl NetworkEndpoint for SimpleNetworkEndpoint {
    async fn network_id(&self) -> Result<u64, NetworkEndpointError> {
        Ok(self.provider.get_chain_id().await?)
    }

    async fn block_height(&self) -> Result<u64, NetworkEndpointError> {
        Ok(self.provider.get_block_number().await?)
    }

    async fn balance(&self, address: Address) -> Result<U256, NetworkEndpointError> {
        Ok(self.provider.get_balance(address).await?)
    }

    async fn code_at(&self, address: Address) -> Result<Bytes, NetworkEndpointError> {
        Ok(self.provider.get_code_at(address).await?)
    }

    async fn transaction_count(&self, address: Address) -> Result<u64, NetworkEndpointError> {
        Ok(self.provider.get_transaction_count(address).await?)
    }

    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, NetworkEndpointError> {
        Ok(self.provider.get_logs(filter).await?)
    }

    async fn call(&self, tx: TransactionRequest) -> Result<Bytes, NetworkEndpointError> {
        Ok(self.provider.call(tx).await?)
    }

    async fn estimate_gas(&self, tx: TransactionRequest) -> Result<u64, NetworkEndpointError> {
        Ok(self.provider.estimate_gas(tx).await?)
    }

    async fn estimate_fees(&self) -> Result<FeeEstimate, NetworkEndpointError> {
        let fees = self.provider.estimate_eip1559_fees().await?;
        Ok(FeeEstimate {
            max_fee_per_gas: fees.max_fee_per_gas,
            max_priority_fee_per_gas: fees.max_priority_fee_per_gas,
        })
    }

    async fn send_transaction(&self, tx: TxEnvelope) -> Result<TxHash, NetworkEndpointError> {
        let pending = self.provider.send_tx_envelope(tx).await?;
        Ok(*pending.tx_hash())
    }

    async fn receipt(
        &self,
        tx: TxHash,
    ) -> Result<Option<TransactionReceipt>, NetworkEndpointError> {
        Ok(self.provider.get_transaction_receipt(tx).await?)
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

impl From<TransportError> for NetworkEndpointError {
    fn from(error: TransportError) -> Self {
        Self::Backend(Box::new(error))
    }
}
