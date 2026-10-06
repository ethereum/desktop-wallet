use std::{num::NonZeroU64, ops::RangeInclusive, time::Duration};

use alloy_consensus::TxEnvelope;
use alloy_eips::eip7702::constants::EIP7702_DELEGATION_DESIGNATOR;
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_rpc_types_eth::{Filter, Log, TransactionReceipt, TransactionRequest};
use async_trait::async_trait;

use super::{NetworkId, logs::span_filters};

#[async_trait]
pub trait NetworkEndpoint: Send + Sync {
    async fn network_id(&self) -> Result<u64, NetworkEndpointError>;
    async fn block_height(&self) -> Result<u64, NetworkEndpointError>;

    async fn balance(&self, address: Address) -> Result<U256, NetworkEndpointError>;
    /// Empty for an account with no code.
    async fn code_at(&self, address: Address) -> Result<Bytes, NetworkEndpointError>;
    /// Also `address`'s next nonce.
    async fn transaction_count(&self, address: Address) -> Result<u64, NetworkEndpointError>;

    /// One request, so a strict endpoint may refuse a wide `filter`. See
    /// [`NetworkEndpoint::logs_in_range`].
    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>, NetworkEndpointError>;

    /// Executes `tx` against the latest block without submitting it.
    async fn call(&self, tx: TransactionRequest) -> Result<Bytes, NetworkEndpointError>;
    async fn estimate_gas(&self, tx: TransactionRequest) -> Result<u64, NetworkEndpointError>;
    async fn estimate_fees(&self) -> Result<FeeEstimate, NetworkEndpointError>;

    /// Acceptance is not inclusion: poll [`NetworkEndpoint::receipt`] for that.
    async fn send_transaction(&self, tx: TxEnvelope) -> Result<TxHash, NetworkEndpointError>;
    /// `None` while `tx` is unmined or unknown.
    async fn receipt(&self, tx: TxHash)
    -> Result<Option<TransactionReceipt>, NetworkEndpointError>;

    /// Rejects this endpoint unless it serves `expected`. Costs one round trip.
    async fn verify_network_id(&self, expected: NetworkId) -> Result<(), NetworkEndpointError> {
        let found = self.network_id().await?;
        if found == expected.0 {
            return Ok(());
        }
        Err(NetworkEndpointError::NetworkMismatch {
            expected: expected.0,
            found,
        })
    }

    /// `tx` sent from `from`, with its nonce, network id, EIP-1559 fees, and gas limit filled
    /// in from this endpoint.
    async fn fill_transaction(
        &self,
        tx: TransactionRequest,
        from: Address,
    ) -> Result<TransactionRequest, NetworkEndpointError> {
        let nonce = self.transaction_count(from).await?;
        let network_id = self.network_id().await?;
        let fees = self.estimate_fees().await?;
        let tx = tx
            .from(from)
            .nonce(nonce)
            .with_chain_id(network_id)
            .with_max_fee_per_gas(fees.max_fee_per_gas)
            .with_max_priority_fee_per_gas(fees.max_priority_fee_per_gas);
        let gas_limit = self.estimate_gas(tx.clone()).await?;
        Ok(tx.with_gas_limit(gas_limit))
    }

    /// Reads `filter` over `blocks` in spans of at most `span`, in ascending block order.
    ///
    /// Costs one request per span. A failing span aborts the read, so a returned `Vec` always
    /// covers the whole range.
    async fn logs_in_range(
        &self,
        filter: &Filter,
        blocks: RangeInclusive<u64>,
        span: NonZeroU64,
    ) -> Result<Vec<Log>, NetworkEndpointError> {
        let mut logs = Vec::new();
        for bounded in span_filters(filter, blocks, span) {
            logs.extend(self.logs(&bounded).await?);
        }
        Ok(logs)
    }

    /// The implementation `address` delegates to under EIP-7702, if it delegates at all.
    async fn delegation_of(
        &self,
        address: Address,
    ) -> Result<Option<Address>, NetworkEndpointError> {
        let code = self.code_at(address).await?;
        Ok(code
            .strip_prefix(EIP7702_DELEGATION_DESIGNATOR.as_slice())
            .filter(|implementation| implementation.len() == Address::len_bytes())
            .map(Address::from_slice))
    }

    /// Polls for `tx`'s receipt every `interval` until `timeout`. `None` if it never arrives.
    async fn await_receipt(
        &self,
        tx: TxHash,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Option<TransactionReceipt>, NetworkEndpointError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(receipt) = self.receipt(tx).await? {
                return Ok(Some(receipt));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Whether `address` has been used: a non-zero nonce, code, or a native balance.
    /// Stops at the first signal.
    ///
    /// ERC-20 receipts do not count yet, so a dust airdrop alone looks unused. A plain
    /// JSON-RPC node cannot list an address's tokens, and every workaround (a token list,
    /// `Transfer` logs from genesis, an indexer) misses tokens or leaks privacy.
    async fn has_activity(&self, address: Address) -> Result<bool, NetworkEndpointError> {
        if self.transaction_count(address).await? != 0 {
            return Ok(true);
        }
        if !self.code_at(address).await?.is_empty() {
            return Ok(true);
        }
        Ok(!self.balance(address).await?.is_zero())
    }
}

/// EIP-1559 fee parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeEstimate {
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkEndpointError {
    /// The inner error type is not part of this API.
    #[error(transparent)]
    Backend(Box<dyn std::error::Error + Send + Sync>),
    #[error("RPC URL `{0}` must be an http or https URL")]
    InvalidUrl(String),
    #[error(
        "endpoint serves network id {found}, but the network is configured as network id {expected}"
    )]
    NetworkMismatch { expected: u64, found: u64 },
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

        mocked_provider(&asserter)
            .verify_network_id(NetworkId(1))
            .await
            .expect("the endpoint serves the network id it was configured as");
    }

    #[tokio::test]
    async fn an_endpoint_serving_another_network_id_is_rejected() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));

        let Err(error) = mocked_provider(&asserter)
            .verify_network_id(NetworkId(11_155_111))
            .await
        else {
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

    #[tokio::test]
    async fn activity_stops_at_a_non_zero_nonce() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));
        let used = mocked_provider(&asserter)
            .has_activity(Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(used);
        assert!(
            asserter.read_q().is_empty(),
            "a non-zero nonce must not fetch code or balance"
        );
    }

    #[tokio::test]
    async fn an_unused_address_is_checked_for_nonce_code_and_balance() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&Bytes::new());
        asserter.push_success(&U256::ZERO);
        let used = mocked_provider(&asserter)
            .has_activity(Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(!used);
        assert!(asserter.read_q().is_empty());
    }

    #[tokio::test]
    async fn code_counts_as_activity() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&Bytes::from_static(&[0xef, 0x01, 0x00]));
        let used = mocked_provider(&asserter)
            .has_activity(Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(used);
        assert!(
            asserter.read_q().is_empty(),
            "code must not fetch balance once it is already used"
        );
    }

    #[tokio::test]
    async fn a_native_balance_counts_as_activity() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&Bytes::new());
        asserter.push_success(&U256::from(1));
        let used = mocked_provider(&asserter)
            .has_activity(Address::repeat_byte(0x11))
            .await
            .unwrap();
        assert!(used);
    }
}
