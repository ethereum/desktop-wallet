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
    simple_signer::{
        db::{SimpleSignerDatabaseError, SimpleSignerDb},
        persist_and_rebuild,
    },
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
    /// The signer is rebuilt from `db` and its [`Signer::tag`] recorded, and `implementation`
    /// is written, before any 7702 authorization is submitted: an authorization that lands
    /// while `db` is missing either one delegates an account this executor cannot rebuild.
    ///
    /// # Errors
    /// Returns an error if the signer cannot be rebuilt from `db`, if the database cannot be
    /// written, or if there is a RPC error.
    pub async fn new_with_implementation(
        signer: Arc<dyn Signer>,
        implementation: Address,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleExecutorError> {
        persist_and_rebuild(
            signer.as_ref(),
            BuildContext::new(provider.clone(), db.clone()),
        )
        .await?;
        db.put_implementation(&implementation).await?;
        Self::authorize_if_missing(implementation, &signer, provider.as_ref()).await?;

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

#[cfg(test)]
mod tests {
    use alloy_primitives::{Bytes, U64};
    use alloy_signer_local::PrivateKeySigner;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{
        database::memory::MemoryDatabase,
        simple_delegate::delegation_designator_code,
        simple_signer::{SIMPLE_SIGNER_TAG, SimpleSigner},
        test_support::mocked_provider,
    };

    /// The address is the one the stored key belongs to.
    async fn seeded_db() -> (Arc<dyn Database>, Address) {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let signer = SimpleSigner::new(PrivateKeySigner::random().credential().clone(), &db)
            .await
            .expect("build signer");
        db.put_signer_tag(signer.tag()).await.expect("store tag");
        db.put_implementation(&SIMPLE_DELEGATE_ADDRESS)
            .await
            .expect("store implementation");
        (db, signer.address())
    }

    #[tokio::test]
    async fn from_context_rebuilds_the_signer_named_by_the_stored_tag() {
        let (db, address) = seeded_db().await;
        let asserter = Asserter::new();
        // The delegation lookup runs twice, once for `authorize_if_missing` and once for the
        // delegate itself, and the delegate then reads the chain id for its EIP-712 domain.
        asserter.push_success(&delegation_designator_code(SIMPLE_DELEGATE_ADDRESS));
        asserter.push_success(&delegation_designator_code(SIMPLE_DELEGATE_ADDRESS));
        asserter.push_success(&U64::from(1));

        let executor =
            SimpleExecutor::from_context(BuildContext::new(mocked_provider(&asserter), db.clone()))
                .await
                .expect("rebuild from the stored tag");

        assert_eq!(
            executor.id(),
            ExecutorId::Address(address),
            "the rebuilt executor must belong to the key the stored tag names",
        );
    }

    /// The ordering [`SimpleExecutor::new_with_implementation`] promises: nothing is
    /// authorized on-chain until the database can rebuild what was authorized.
    #[tokio::test]
    async fn the_database_is_written_before_any_authorization_is_attempted() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let signer: Arc<dyn Signer> = Arc::new(
            SimpleSigner::new(PrivateKeySigner::random().credential().clone(), &db)
                .await
                .expect("build signer"),
        );
        let asserter = Asserter::new();
        // Undelegated, so the build goes on to authorize and runs out of queued responses.
        asserter.push_success(&Bytes::new());

        let result = SimpleExecutor::new_with_implementation(
            signer,
            SIMPLE_DELEGATE_ADDRESS,
            mocked_provider(&asserter),
            db.clone(),
        )
        .await;

        assert!(result.is_err(), "the authorization has no chain to land on");
        assert_eq!(
            db.get_signer_tag().await.expect("tag stored"),
            SIMPLE_SIGNER_TAG,
        );
        db.get_implementation()
            .await
            .expect("implementation stored");
    }

    #[tokio::test]
    async fn from_context_fails_before_the_chain_when_no_tag_is_stored() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let asserter = Asserter::new();
        asserter.push_success(&delegation_designator_code(SIMPLE_DELEGATE_ADDRESS));

        let result =
            SimpleExecutor::from_context(BuildContext::new(mocked_provider(&asserter), db)).await;

        let Err(error) = result else {
            panic!("there is nothing to rebuild from");
        };
        assert!(
            matches!(
                error,
                SimpleExecutorError::SignerDatabase(SimpleSignerDatabaseError::MissingSignerTag)
            ),
            "expected a missing tag to be reported as such, got {error}",
        );
        assert_eq!(
            asserter.read_q().len(),
            1,
            "a build with no signer to rebuild must not reach the chain",
        );
    }

    #[tokio::test]
    async fn from_context_fails_before_the_chain_when_the_tag_names_an_unrebuildable_signer() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        db.put_signer_tag(SIMPLE_SIGNER_TAG)
            .await
            .expect("store tag");
        db.put_implementation(&SIMPLE_DELEGATE_ADDRESS)
            .await
            .expect("store implementation");
        let asserter = Asserter::new();
        asserter.push_success(&delegation_designator_code(SIMPLE_DELEGATE_ADDRESS));

        let result =
            SimpleExecutor::from_context(BuildContext::new(mocked_provider(&asserter), db)).await;

        let Err(error) = result else {
            panic!("the tag names a signer whose key was never stored");
        };
        assert!(
            matches!(error, SimpleExecutorError::Factory(_)),
            "expected the factory to refuse, got {error}",
        );
        assert_eq!(
            asserter.read_q().len(),
            1,
            "a build whose signer cannot be rebuilt must not reach the chain",
        );
    }
}
