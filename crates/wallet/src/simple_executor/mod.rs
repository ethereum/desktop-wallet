use std::sync::Arc;

use alloy_consensus::TxEnvelope;
use alloy_network::{
    NetworkTransactionBuilder, TransactionBuilder, TransactionBuilder7702, TxSigner,
};
use alloy_primitives::{Address, B256};
use alloy_provider::{Provider, network::EthereumWallet};
use alloy_rpc_types_eth::TransactionRequest;
use alloy_signer_local::PrivateKeySigner;
use edw_core::{
    call::Call,
    database::Database,
    executor::{CallId, CallReceipt, Executor, ExecutorError, ExecutorId},
    factory::{BuildContext, Factory, FactoryError},
};
use tracing::info;

use crate::{
    simple_delegate::{SIMPLE_DELEGATE_ADDRESS, SimpleDelegate, SimpleDelegateError, is_delegated},
    simple_executor::db::{SimpleExecutorDatabaseError, SimpleExecutorDb},
};

mod db;

/// `SimpleExecutor` is a basic [`Executor`] implementation that uses an signer-based
/// wallet to execute calls through the `SimpleDelegate` contract.
pub struct SimpleExecutor {
    delegate: SimpleDelegate<PrivateKeySigner>,
    wallet: EthereumWallet,
    provider: Arc<dyn Provider>,
    #[allow(unused)]
    db: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleExecutorError {
    #[error(transparent)]
    Delegate(#[from] SimpleDelegateError),
    #[error("database error: {0}")]
    Database(#[from] SimpleExecutorDatabaseError),
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy_transport::RpcError<alloy_transport::TransportErrorKind>),
    #[error("transaction builder error: {0}")]
    Builder(#[from] alloy_network::TransactionBuilderError<alloy_network::Ethereum>),
    #[error("pending transaction error: {0}")]
    Pending(#[from] alloy_provider::PendingTransactionError),
    #[error("transaction failed with status code")]
    TransactionFailed,
}

const SIMPLE_EXECUTOR_TAG: &str = "simple-executor";

inventory::submit! {
    Factory::new(SIMPLE_EXECUTOR_TAG, |ctx: BuildContext| {
        Box::pin(async move {
            SimpleExecutor::from_context(ctx).await.map_err(|e| FactoryError::Other(Box::new(e)))
        })
    })
}

impl SimpleExecutor {
    /// Creates a new `SimpleExecutor`. If the signer has not already delegated to
    /// the `SimpleDelegate` implementation contract, this method will automatically
    /// submit the 7702 authorization.
    ///
    /// # Errors
    /// Returns an error if there is a RPC error.
    pub async fn new(
        signer: PrivateKeySigner,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleExecutorError> {
        Self::new_with_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider, db).await
    }

    /// Creates a new `SimpleExecutor` instance.
    ///
    /// # Errors
    /// Returns an error if there is a RPC error.
    pub async fn new_with_implementation(
        signer: PrivateKeySigner,
        implementation: Address,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleExecutorError> {
        Self::authorize_if_missing(implementation, &signer, provider.as_ref()).await?;
        db.put_signing_key(signer.credential()).await?;
        db.put_implementation(&implementation).await?;

        let delegate = SimpleDelegate::new_with_implementation(
            signer.clone(),
            implementation,
            provider.clone(),
        )
        .await?;

        let wallet = EthereumWallet::new(signer);
        Ok(Self {
            delegate,
            wallet,
            provider,
            db,
        })
    }

    /// Creates a new `SimpleExecutor` instance from the given context.
    ///
    /// # Errors
    /// Errors if the signer cannot be retrieved from the database or if the `SimpleExecutor` cannot
    /// be created (see [`SimpleExecutor::new`]).
    pub async fn from_context(ctx: BuildContext) -> Result<Box<dyn Executor>, SimpleExecutorError> {
        let provider = ctx.provider;
        let db = ctx.db;

        let signing_key = db.get_signing_key().await?;
        let signer = PrivateKeySigner::from_signing_key(signing_key);
        let implementation = db.get_implementation().await?;
        let executor =
            SimpleExecutor::new_with_implementation(signer, implementation, provider, db).await?;
        Ok(Box::new(executor))
    }

    /// Submits the 7702 authorization transaction on-chain if the delegate is not already authorized
    /// for the signer's address.
    async fn authorize_if_missing(
        implementation: Address,
        signer: &PrivateKeySigner,
        provider: &dyn Provider,
    ) -> Result<(), SimpleExecutorError> {
        let delegator = TxSigner::address(&signer);
        if is_delegated(delegator, implementation, provider).await? {
            return Ok(());
        }

        info!(
            "Authorization missing for delegate {implementation:} and signer {delegator:}, authorizing...",
        );

        //? nonce + 1 to account for the authorization transaction
        let nonce = provider.get_transaction_count(delegator).await? + 1;
        let auth =
            SimpleDelegate::authorize_implementation(signer, nonce, provider, implementation)
                .await?;

        let tx = TransactionRequest::default()
            .to(Address::ZERO)
            .with_authorization_list(vec![auth]);
        let wallet = EthereumWallet::new(signer.clone());
        let envelope = fill_and_sign(tx, provider, &wallet).await?;

        let _ = provider
            .send_tx_envelope(envelope)
            .await?
            .get_receipt()
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Executor for SimpleExecutor {
    fn tag(&self) -> &'static str {
        SIMPLE_EXECUTOR_TAG
    }

    fn id(&self) -> ExecutorId {
        ExecutorId::Address(self.address())
    }

    fn address(&self) -> Address {
        self.delegate.address()
    }

    async fn execute(&self, calls: &[Call]) -> Result<CallId, ExecutorError> {
        let txhash = self.send(calls).await?;
        Ok(CallId(txhash))
    }

    async fn receipt(&self, id: CallId) -> Result<Option<CallReceipt>, ExecutorError> {
        let hash = id.0;
        let receipt = self
            .provider
            .get_transaction_receipt(hash)
            .await
            .map_err(|e| ExecutorError::Other(Box::new(e)))?;

        let Some(_receipt) = receipt else {
            return Ok(None);
        };

        // TODO
        Ok(Some(CallReceipt))
    }
}

impl SimpleExecutor {
    async fn send(&self, calls: &[Call]) -> Result<B256, SimpleExecutorError> {
        let call = self.delegate.batch_calls(calls).await?;

        let tx = TransactionRequest::default()
            .to(call.target)
            .value(call.value)
            .with_input(call.data);
        let envelope = fill_and_sign(tx, self.provider.as_ref(), &self.wallet).await?;

        let pending_tx = self.provider.send_tx_envelope(envelope).await?;
        Ok(*pending_tx.tx_hash())
    }
}

/// Fills the transaction's nonce, chain ID, gas limit, and fee parameters, then
/// signs it with the wallet.
async fn fill_and_sign(
    tx: TransactionRequest,
    provider: &dyn Provider,
    wallet: &EthereumWallet,
) -> Result<TxEnvelope, SimpleExecutorError> {
    let from = wallet.default_signer().address();
    let nonce = provider.get_transaction_count(from).await?;
    let chain_id = provider.get_chain_id().await?;
    let fees = provider.estimate_eip1559_fees().await?;

    let tx = tx
        .from(from)
        .nonce(nonce)
        .with_chain_id(chain_id)
        .with_max_fee_per_gas(fees.max_fee_per_gas)
        .with_max_priority_fee_per_gas(fees.max_priority_fee_per_gas);

    let gas_limit = provider.estimate_gas(tx.clone()).await?;
    let tx = tx.with_gas_limit(gas_limit);

    let tx_envelope = tx.build(wallet).await?;
    Ok(tx_envelope)
}

impl From<SimpleExecutorError> for ExecutorError {
    fn from(err: SimpleExecutorError) -> Self {
        ExecutorError::Other(Box::new(err))
    }
}
