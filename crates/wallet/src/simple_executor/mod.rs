use std::sync::Arc;

use alloy_consensus::{SignableTransaction, TxEnvelope};
use alloy_network::{
    NetworkTransactionBuilder, TransactionBuilder, TransactionBuilder7702, TxSigner,
};
use alloy_primitives::{Address, B256, Signature};
use alloy_provider::{Provider, network::EthereumWallet};
use alloy_rpc_types_eth::TransactionRequest;
use edw_core::{
    call::Call,
    database::Database,
    executor::{CallId, CallReceipt, Executor, ExecutorError, ExecutorId},
    factory::{BuildContext, Factory, FactoryError, try_build_signer},
    signer::Signer,
};
use tracing::info;

use crate::{
    simple_delegate::{
        SIMPLE_DELEGATE_ADDRESS, SimpleDelegate, SimpleDelegateError, is_delegated, signer_address,
    },
    simple_executor::db::{SimpleExecutorDatabaseError, SimpleExecutorDb},
    simple_signer::db::{SimpleSignerDatabaseError, SimpleSignerDb},
};

pub(crate) mod db;

/// `SimpleExecutor` is a basic [`Executor`] implementation that uses an signer-based
/// wallet to execute calls through the `SimpleDelegate` contract.
pub struct SimpleExecutor {
    delegate: SimpleDelegate,
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
    #[error("signer database error: {0}")]
    SignerDatabase(#[from] SimpleSignerDatabaseError),
    #[error("factory error: {0}")]
    Factory(#[from] FactoryError),
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
        signer: Arc<dyn Signer>,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleExecutorError> {
        Self::new_with_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider, db).await
    }

    /// Creates a new `SimpleExecutor` instance.
    ///
    /// `signer`'s [`Signer::tag`] is recorded so that [`SimpleExecutor::from_context`] rebuilds
    /// the same kind through the factory, which requires `signer` to have stored whatever it
    /// needs in `db`.
    ///
    /// # Errors
    /// Returns an error if there is a RPC error.
    pub async fn new_with_implementation(
        signer: Arc<dyn Signer>,
        implementation: Address,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleExecutorError> {
        Self::authorize_if_missing(implementation, &signer, provider.as_ref()).await?;
        db.put_signer_tag(signer.tag()).await?;
        db.put_implementation(&implementation).await?;

        let delegate = SimpleDelegate::new_with_implementation(
            signer.clone(),
            implementation,
            provider.clone(),
        )
        .await?;

        let wallet = EthereumWallet::new(TxSignerBridge::new(signer));
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
        let tag = ctx.db.get_signer_tag().await?;
        let signer: Arc<dyn Signer> = Arc::from(try_build_signer(&tag, ctx.clone()).await?);
        let provider = ctx.provider;
        let db = ctx.db;

        let implementation = db.get_implementation().await?;
        let executor =
            SimpleExecutor::new_with_implementation(signer, implementation, provider, db).await?;
        Ok(Box::new(executor))
    }

    /// Submits the 7702 authorization transaction on-chain if the delegate is not already authorized
    /// for the signer's address.
    async fn authorize_if_missing(
        implementation: Address,
        signer: &Arc<dyn Signer>,
        provider: &dyn Provider,
    ) -> Result<(), SimpleExecutorError> {
        let delegator = signer_address(signer.as_ref());
        if is_delegated(delegator, implementation, provider).await? {
            return Ok(());
        }

        info!(
            "Authorization missing for delegate {implementation:} and signer {delegator:}, authorizing...",
        );

        //? nonce + 1 to account for the authorization transaction
        let nonce = provider.get_transaction_count(delegator).await? + 1;
        let auth = SimpleDelegate::authorize_implementation(
            signer.as_ref(),
            nonce,
            provider,
            implementation,
        )
        .await?;

        let tx = TransactionRequest::default()
            .to(Address::ZERO)
            .with_authorization_list(vec![auth]);
        let wallet = EthereumWallet::new(TxSignerBridge::new(signer.clone()));
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

/// [`EthereumWallet`] needs a [`TxSigner`], so the provider's fill-and-sign path reaches this
/// executor's signer through here.
struct TxSignerBridge {
    signer: Arc<dyn Signer>,
    address: Address,
}

impl TxSignerBridge {
    fn new(signer: Arc<dyn Signer>) -> Self {
        let address = signer_address(signer.as_ref());
        Self { signer, address }
    }
}

#[async_trait::async_trait]
impl TxSigner<Signature> for TxSignerBridge {
    fn address(&self) -> Address {
        self.address
    }

    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature> {
        self.signer
            .sign_transaction(tx)
            .await
            .map_err(alloy_signer::Error::other)
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
