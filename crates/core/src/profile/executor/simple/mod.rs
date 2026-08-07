use alloy::{
    consensus::TxEnvelope,
    network::{
        EthereumWallet, NetworkTransactionBuilder, TransactionBuilder, TransactionBuilder7702,
    },
    primitives::Address,
    providers::Provider,
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use tracing::info;

use crate::{
    call::Call,
    profile::{
        ExecutorId,
        executor::{Executor, ExecutorError},
    },
    simple_delegate::{SIMPLE_DELEGATE_ADDRESS, SimpleDelegate, SimpleDelegateError},
};

pub struct SimpleExecutor<P: Provider> {
    delegate: SimpleDelegate<P>,
    wallet: EthereumWallet,
    provider: P,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleExecutorError {
    #[error(transparent)]
    Delegate(#[from] SimpleDelegateError),
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),
    #[error("transaction builder error: {0}")]
    Builder(#[from] alloy::network::TransactionBuilderError<alloy::network::Ethereum>),
    #[error("pending transaction error: {0}")]
    Pending(#[from] alloy::providers::PendingTransactionError),
    #[error("transaction failed with status code")]
    TransactionFailed,
}

impl<P: Provider + Clone> SimpleExecutor<P> {
    pub async fn new(signer: PrivateKeySigner, provider: P) -> Result<Self, SimpleExecutorError> {
        Self::new_with_delegate(signer, provider, SIMPLE_DELEGATE_ADDRESS).await
    }

    pub async fn new_with_delegate(
        signer: PrivateKeySigner,
        provider: P,
        delegate: Address,
    ) -> Result<Self, SimpleExecutorError> {
        Self::authorize_if_missing(delegate, &signer, &provider).await?;

        let delegate =
            SimpleDelegate::new_with_delegate(signer.clone(), provider.clone(), delegate).await?;
        let wallet = EthereumWallet::new(signer);
        Ok(Self {
            delegate,
            wallet,
            provider,
        })
    }

    /// Submits the 7702 authorization transaction on-chain if the delegate is not already authorized
    /// for the signer's address.
    async fn authorize_if_missing(
        delegate: Address,
        signer: &PrivateKeySigner,
        provider: &P,
    ) -> Result<(), SimpleExecutorError> {
        let address = signer.address();
        if SimpleDelegate::authorized(address, delegate, provider).await? {
            return Ok(());
        }

        info!(
            "Authorization missing for delegate {delegate:} and signer {address:}, authorizing...",
        );

        //? nonce + 1 to account for the authorization transaction itself
        let nonce = provider.get_transaction_count(signer.address()).await? + 1;
        let auth = SimpleDelegate::authorize_delegate(signer, nonce, provider, delegate).await?;

        let tx = TransactionRequest::default()
            .to(Address::ZERO)
            .with_authorization_list(vec![auth]);
        let wallet = EthereumWallet::new(signer.clone());
        let envelope = fill_and_sign(tx, provider, &wallet).await?;

        let _ = send_signed(envelope, provider).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<P: Provider> Executor for SimpleExecutor<P> {
    fn id(&self) -> ExecutorId {
        ExecutorId::Address(self.address())
    }

    fn address(&self) -> Address {
        self.delegate.address()
    }

    async fn execute(&self, calls: &[Call]) -> Result<(), ExecutorError> {
        self.send(calls).await?;
        Ok(())
    }
}

impl<P: Provider> SimpleExecutor<P> {
    async fn send(&self, calls: &[Call]) -> Result<(), SimpleExecutorError> {
        let call = self.delegate.batch_calls(calls).await?;

        let tx = TransactionRequest::default()
            .to(call.target)
            .value(call.value)
            .with_input(call.data);
        let envelope = fill_and_sign(tx, &self.provider, &self.wallet).await?;
        dbg!(&envelope);

        let _ = send_signed(envelope, &self.provider).await?;
        Ok(())
    }
}

/// Fills the transaction's nonce, chain ID, gas limit, and fee parameters, then
/// signs it with the wallet.
async fn fill_and_sign<P: Provider>(
    tx: TransactionRequest,
    provider: &P,
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

    dbg!(&tx);
    let gas_limit = provider.estimate_gas(tx.clone()).await?;
    let tx = tx.with_gas_limit(gas_limit);

    let tx_envelope = tx.build(wallet).await?;
    Ok(tx_envelope)
}

async fn send_signed<P: Provider>(
    envelope: TxEnvelope,
    provider: &P,
) -> Result<TransactionReceipt, SimpleExecutorError> {
    let receipt = provider
        .send_tx_envelope(envelope)
        .await?
        .get_receipt()
        .await?;

    if !receipt.status() {
        return Err(SimpleExecutorError::TransactionFailed);
    }

    Ok(receipt)
}

impl From<SimpleExecutorError> for ExecutorError {
    fn from(err: SimpleExecutorError) -> Self {
        ExecutorError::Inner(Box::new(err))
    }
}
