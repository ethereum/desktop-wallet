use std::{num::NonZeroU64, sync::Arc, time::Duration};

use alloy_consensus::TxEnvelope;
use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_rpc_types_eth::{Filter, Log, TransactionReceipt, TransactionRequest};
use alloy_transport::TransportError;
use async_trait::async_trait;
use reqwest::Url;
use tokio::sync::Mutex;

use super::{
    DEFAULT_EVENT_BLOCK_RANGE,
    endpoint::{FeeEstimate, NetworkEndpoint, NetworkEndpointError},
    http::outbound_http,
};

#[derive(Debug, Clone)]
pub struct SimpleNetworkEndpoint {
    provider: DynProvider,
    event_block_range: NonZeroU64,
    pace: Arc<Pace>,
}

#[derive(Debug)]
struct Pace {
    interval: Duration,
    next: Mutex<tokio::time::Instant>,
}

struct PaceGuard<'a> {
    slot: Option<tokio::sync::MutexGuard<'a, tokio::time::Instant>>,
    interval: Duration,
}

impl Drop for PaceGuard<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.as_mut() {
            **slot = tokio::time::Instant::now() + self.interval;
        }
    }
}

impl Pace {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: Mutex::new(tokio::time::Instant::now()),
        }
    }

    async fn enter(&self) -> PaceGuard<'_> {
        if self.interval.is_zero() {
            return PaceGuard {
                slot: None,
                interval: Duration::ZERO,
            };
        }
        let slot = self.next.lock().await;
        let now = tokio::time::Instant::now();
        if *slot > now {
            tokio::time::sleep_until(*slot).await;
        }
        PaceGuard {
            slot: Some(slot),
            interval: self.interval,
        }
    }
}

#[async_trait]
impl NetworkEndpoint for SimpleNetworkEndpoint {
    fn event_block_range(&self) -> NonZeroU64 {
        self.event_block_range
    }

    async fn network_id(&self) -> Result<u64, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_chain_id().await?)
    }

    async fn block_height(&self) -> Result<u64, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_block_number().await?)
    }

    async fn balance(&self, address: Address) -> Result<U256, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_balance(address).await?)
    }

    async fn code_at(&self, address: Address) -> Result<Bytes, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_code_at(address).await?)
    }

    async fn transaction_count(&self, address: Address) -> Result<u64, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_transaction_count(address).await?)
    }

    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_logs(filter).await?)
    }

    async fn call(&self, tx: TransactionRequest) -> Result<Bytes, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.call(tx).await?)
    }

    async fn estimate_gas(&self, tx: TransactionRequest) -> Result<u64, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.estimate_gas(tx).await?)
    }

    async fn estimate_fees(&self) -> Result<FeeEstimate, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        let fees = self.provider.estimate_eip1559_fees().await?;
        Ok(FeeEstimate {
            max_fee_per_gas: fees.max_fee_per_gas,
            max_priority_fee_per_gas: fees.max_priority_fee_per_gas,
        })
    }

    async fn send_transaction(&self, tx: TxEnvelope) -> Result<TxHash, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        let pending = self.provider.send_tx_envelope(tx).await?;
        Ok(*pending.tx_hash())
    }

    async fn receipt(
        &self,
        tx: TxHash,
    ) -> Result<Option<TransactionReceipt>, NetworkEndpointError> {
        let _pace = self.pace.enter().await;
        Ok(self.provider.get_transaction_receipt(tx).await?)
    }

    fn provider(&self) -> Option<DynProvider> {
        Some(self.provider.clone())
    }
}

impl SimpleNetworkEndpoint {
    #[must_use]
    pub fn new(provider: DynProvider) -> Self {
        Self::with_limits(provider, DEFAULT_EVENT_BLOCK_RANGE, Duration::ZERO)
    }

    /// `event_block_range` is how many blocks one log request may cover. `call_interval` is the
    /// pause after one request before the next. Zero waits nothing.
    #[must_use]
    pub fn with_limits(
        provider: DynProvider,
        event_block_range: NonZeroU64,
        call_interval: Duration,
    ) -> Self {
        Self {
            provider,
            event_block_range,
            pace: Arc::new(Pace::new(call_interval)),
        }
    }

    /// Connects with the shared [`outbound_http`] client.
    ///
    /// # Errors
    /// Returns an error if the HTTP client cannot be built.
    pub fn new_http(
        url: Url,
        event_block_range: NonZeroU64,
        call_interval: Duration,
    ) -> Result<Self, NetworkEndpointError> {
        let client = outbound_http()
            .build()
            .map_err(|error| NetworkEndpointError::Backend(Box::new(error)))?;
        Ok(Self::with_limits(
            ProviderBuilder::new().connect_reqwest(client, url).erased(),
            event_block_range,
            call_interval,
        ))
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
