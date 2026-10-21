use std::{sync::Arc, time::Duration};

use alloy_network::{
    EthereumWallet, NetworkTransactionBuilder, TransactionBuilder, TransactionBuilder7702,
};
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types_eth::TransactionRequest;

use crate::{
    call::Call,
    delegate::simple::{SIMPLE_DELEGATE_ADDRESS, SimpleDelegate},
    network::endpoint::NetworkEndpointError,
    prelude::*,
    signer::{Signer, SignerError},
};

/// How long to wait for a submitted transaction to be mined.
const INCLUSION_TIMEOUT: Duration = Duration::from_mins(2);
const INCLUSION_POLL: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum BroadcastError {
    #[error("network error: {0}")]
    Network(#[from] NetworkEndpointError),
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("delegate error: {0}")]
    Delegate(#[from] crate::delegate::simple::SimpleDelegateError),
    #[error("transaction builder error: {0}")]
    Builder(#[from] alloy_network::TransactionBuilderError<alloy_network::Ethereum>),
    #[error("transaction {tx} was not mined within {timeout:?}")]
    NotMined { tx: TxHash, timeout: Duration },
    #[error("transaction {tx} reverted")]
    Reverted { tx: TxHash },
}

/// Sends one transaction and waits until it is mined.
///
/// # Errors
/// Returns an error if the endpoint rejects the transaction, it is not mined in time, or it
/// reverts.
pub async fn submit_call(
    endpoint: &dyn NetworkEndpoint,
    signer: Arc<dyn Signer>,
    call: &Call,
) -> Result<TxHash, BroadcastError> {
    let tx = TransactionRequest::default()
        .to(call.target)
        .value(call.value)
        .with_input(call.data.clone());
    let hash = send(endpoint, &signer, tx).await?;
    wait_mined(endpoint, hash).await?;
    Ok(hash)
}

/// Runs `calls` as one [`SimpleDelegate`] batch signed by `context`.
///
/// `payer` submits the transaction and pays for gas. When `context` is not yet delegated,
/// the same transaction carries its 7702 authorization, except when `payer` and `context`
/// are the same account: that authorization is its own transaction, because the account's
/// nonce advances before the authorization list is applied.
///
/// # Errors
/// Returns an error if authorization, submission, or inclusion fails.
pub async fn submit_batch(
    endpoint: &dyn NetworkEndpoint,
    context: Arc<dyn Signer>,
    payer: Arc<dyn Signer>,
    calls: &[Call],
) -> Result<TxHash, BroadcastError> {
    let network_id = endpoint.network_id().await?;
    let delegated = endpoint.delegation_of(context.address()).await?;
    let installed = delegated == Some(SIMPLE_DELEGATE_ADDRESS);

    if !installed && payer.address() == context.address() {
        authorize_self(endpoint, &context).await?;
    }

    let nonce = if installed || payer.address() == context.address() {
        SimpleDelegate::on_chain_nonce(endpoint, context.address()).await?
    } else {
        U256::ZERO
    };
    let batch = SimpleDelegate::signed_batch(context.as_ref(), network_id, nonce, calls).await?;
    let mut tx = TransactionRequest::default()
        .to(batch.target)
        .value(batch.value)
        .with_input(batch.data);
    if !installed && payer.address() != context.address() {
        let auth_nonce = endpoint.transaction_count(context.address()).await?;
        let auth = SimpleDelegate::authorize_implementation(
            context.as_ref(),
            auth_nonce,
            endpoint,
            SIMPLE_DELEGATE_ADDRESS,
        )
        .await?;
        tx = tx.with_authorization_list(vec![auth]);
    }

    let hash = send(endpoint, &payer, tx).await?;
    wait_mined(endpoint, hash).await?;
    Ok(hash)
}

async fn authorize_self(
    endpoint: &dyn NetworkEndpoint,
    signer: &Arc<dyn Signer>,
) -> Result<(), BroadcastError> {
    let nonce = endpoint.transaction_count(signer.address()).await? + 1;
    let auth = SimpleDelegate::authorize_implementation(
        signer.as_ref(),
        nonce,
        endpoint,
        SIMPLE_DELEGATE_ADDRESS,
    )
    .await?;
    let tx = TransactionRequest::default()
        .to(Address::ZERO)
        .with_authorization_list(vec![auth]);
    let hash = send(endpoint, signer, tx).await?;
    wait_mined(endpoint, hash).await?;
    Ok(())
}

async fn send(
    endpoint: &dyn NetworkEndpoint,
    signer: &Arc<dyn Signer>,
    tx: TransactionRequest,
) -> Result<TxHash, BroadcastError> {
    let wallet = EthereumWallet::new(Arc::clone(signer));
    let envelope = endpoint
        .fill_transaction(tx, signer.address())
        .await?
        .build(&wallet)
        .await?;
    Ok(endpoint.send_transaction(envelope).await?)
}

async fn wait_mined(endpoint: &dyn NetworkEndpoint, tx: TxHash) -> Result<(), BroadcastError> {
    let Some(receipt) = endpoint
        .await_receipt(tx, INCLUSION_TIMEOUT, INCLUSION_POLL)
        .await?
    else {
        return Err(BroadcastError::NotMined {
            tx,
            timeout: INCLUSION_TIMEOUT,
        });
    };
    if receipt.status() {
        Ok(())
    } else {
        Err(BroadcastError::Reverted { tx })
    }
}
