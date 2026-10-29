use std::{sync::Arc, time::Duration};

use alloy_network::{
    EthereumWallet, NetworkTransactionBuilder, TransactionBuilder, TransactionBuilder7702,
};
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types_eth::{TransactionReceipt, TransactionRequest};
use alloy_signer::Signer as AlloySigner;
use alloy_signer_local::PrivateKeySigner;
use kohaku_userop_kit::{
    builder::UserOperationBuilder,
    bundler::Bundler,
    smart_account::simple_7702_smart_account::{Call as UserOpCall, Simple7702SmartAccount},
    user_operation::{
        Authorization as UserOpAuthorization, UserOperation, UserOperationGasEstimate,
    },
};

use crate::{
    bundler::{BundlerList, BundlerListError, Url},
    call::Call,
    delegate::simple::{SIMPLE_DELEGATE_ADDRESS, SimpleDelegate},
    network::endpoint::NetworkEndpointError,
    prelude::*,
    signer::{Signer, SignerError},
};

/// How long to wait for a submitted transaction to be mined.
const INCLUSION_TIMEOUT: Duration = Duration::from_mins(2);
const INCLUSION_POLL: Duration = Duration::from_secs(1);

/// A user operation the bundler accepted and a block included.
pub struct SubmittedUserOperation {
    pub hash: String,
    pub transaction: String,
    pub bundler: Option<String>,
}

/// A transaction filled in from the endpoint, not yet signed.
pub struct PreparedTransaction {
    pub request: TransactionRequest,
}

/// A mined transaction that succeeded, and what its gas cost.
pub struct IncludedTransaction {
    pub hash: TxHash,
    pub fee: U256,
}

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
    #[error("transaction from {from} cannot be signed by {signer}")]
    WrongSigner { from: Address, signer: Address },
    #[error("a user operation needs at least one call")]
    EmptyUserOperation,
    #[error("this endpoint cannot submit a user operation")]
    NoProvider,
    #[error("smart account: {0}")]
    Account(String),
    #[error("user operation signature: {0}")]
    Signature(String),
    #[error(transparent)]
    Bundler(#[from] kohaku_userop_kit::bundler::BundlerError),
    #[error(transparent)]
    BundlerList(#[from] BundlerListError),
    #[error("user operation {hash} was accepted but its receipt failed: {message}")]
    UserOperationReceipt { hash: String, message: String },
    #[error("user operation {user_op} was included in {transaction} and reverted")]
    UserOperationReverted {
        user_op: String,
        transaction: String,
    },
}

impl PreparedTransaction {
    #[must_use]
    pub fn gas_limit(&self) -> u64 {
        self.request.gas.unwrap_or_default()
    }

    #[must_use]
    pub fn max_fee_per_gas(&self) -> u128 {
        self.request.max_fee_per_gas.unwrap_or_default()
    }

    /// The most the gas can cost. The node refuses the transaction unless the sender holds this
    /// plus [`Self::value`].
    #[must_use]
    pub fn max_fee(&self) -> U256 {
        U256::from(self.gas_limit()) * U256::from(self.max_fee_per_gas())
    }

    #[must_use]
    pub fn value(&self) -> U256 {
        self.request.value.unwrap_or_default()
    }
}

/// Fills in `call` sent from `from`: nonce, network id, EIP-1559 fees and gas limit.
///
/// # Errors
/// Returns an error if the endpoint fails, or gas estimation reverts.
pub async fn prepare_call(
    endpoint: &dyn NetworkEndpoint,
    from: Address,
    call: &Call,
) -> Result<PreparedTransaction, BroadcastError> {
    let tx = TransactionRequest::default()
        .to(call.target)
        .value(call.value)
        .with_input(call.data.clone());
    Ok(PreparedTransaction {
        request: endpoint.fill_transaction(tx, from).await?,
    })
}

/// Signs `prepared` with `signer` and waits until it is mined.
///
/// # Errors
/// Returns an error if `signer` is not the transaction's sender, the endpoint rejects the
/// transaction, it is not mined in time, or it reverts.
pub async fn submit_prepared(
    endpoint: &dyn NetworkEndpoint,
    signer: Arc<dyn Signer>,
    prepared: PreparedTransaction,
) -> Result<IncludedTransaction, BroadcastError> {
    let from = prepared.request.from.unwrap_or_default();
    if from != signer.address() {
        return Err(BroadcastError::WrongSigner {
            from,
            signer: signer.address(),
        });
    }
    let hash = send_filled(endpoint, &signer, prepared.request).await?;
    let receipt = wait_mined(endpoint, hash).await?;
    Ok(IncludedTransaction {
        hash,
        fee: U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price),
    })
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
    let prepared = prepare_call(endpoint, signer.address(), call).await?;
    Ok(submit_prepared(endpoint, signer, prepared).await?.hash)
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
    let filled = endpoint.fill_transaction(tx, signer.address()).await?;
    send_filled(endpoint, signer, filled).await
}

async fn send_filled(
    endpoint: &dyn NetworkEndpoint,
    signer: &Arc<dyn Signer>,
    filled: TransactionRequest,
) -> Result<TxHash, BroadcastError> {
    let wallet = EthereumWallet::new(Arc::clone(signer));
    let envelope = filled.build(&wallet).await?;
    Ok(endpoint.send_transaction(envelope).await?)
}

async fn wait_mined(
    endpoint: &dyn NetworkEndpoint,
    tx: TxHash,
) -> Result<TransactionReceipt, BroadcastError> {
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
        Ok(receipt)
    } else {
        Err(BroadcastError::Reverted { tx })
    }
}

/// Sends `calls` as one user operation from `signer`'s account.
///
/// The account pays gas from its own balance. The operation carries an EIP-7702 authorization
/// for the Simple7702 account when that delegation is not already installed.
/// `bundler_urls` are tried in order; pass the wallet's configured bundler URLs.
///
/// # Errors
/// Returns an error when `calls` is empty, the endpoint has no provider, no bundler is
/// configured, the bundler rejects the operation, or the included operation reverts.
pub async fn submit_user_operation(
    endpoint: &dyn NetworkEndpoint,
    signer: &PrivateKeySigner,
    calls: &[Call],
    bundler_urls: &[Url],
) -> Result<SubmittedUserOperation, BroadcastError> {
    if calls.is_empty() {
        return Err(BroadcastError::EmptyUserOperation);
    }
    let Some(provider) = endpoint.provider() else {
        return Err(BroadcastError::NoProvider);
    };
    let chain_id = endpoint.network_id().await?;
    let owner = AlloySigner::address(signer);
    let account = Simple7702SmartAccount::new(provider, owner, chain_id);
    let user_calls = calls
        .iter()
        .map(|call| UserOpCall {
            target: call.target,
            value: call.value,
            data: call.data.clone(),
        })
        .collect::<Vec<_>>();
    let mut operation = UserOperationBuilder::new_with_smart_account(&account)
        .await
        .map_err(|error| BroadcastError::Account(error.to_string()))?
        .with_call(&user_calls)
        .build();
    if already_delegated(endpoint, owner, &operation.user_op).await? {
        operation.user_op.authorization = UserOpAuthorization::None;
    }
    let bundlers = BundlerList::from_urls(chain_id, bundler_urls)?;
    let gas = bundlers.estimate_gas(&operation, None).await?;
    apply_gas(&mut operation.user_op, gas);
    let signed_operation = operation
        .sign(signer)
        .await
        .map_err(|error| BroadcastError::Signature(error.to_string()))?;
    let hash = bundlers.send_user_operation(&signed_operation).await?;
    let bundler = bundlers.accepted_url();
    let receipt = bundlers.wait_for_receipt(hash).await.map_err(|error| {
        BroadcastError::UserOperationReceipt {
            hash: format!("{:#x}", hash.0),
            message: error.to_string(),
        }
    })?;
    let transaction = receipt.receipt.transaction_hash;
    if !receipt.success {
        return Err(BroadcastError::UserOperationReverted {
            user_op: format!("{:#x}", hash.0),
            transaction: format!("{transaction:#x}"),
        });
    }
    Ok(SubmittedUserOperation {
        hash: format!("{:#x}", hash.0),
        transaction: format!("{transaction:#x}"),
        bundler,
    })
}

async fn already_delegated(
    endpoint: &dyn NetworkEndpoint,
    owner: Address,
    operation: &UserOperation,
) -> Result<bool, BroadcastError> {
    let UserOpAuthorization::Eip7702(authorization) = &operation.authorization else {
        return Err(BroadcastError::Account(
            "missing 7702 authorization".to_string(),
        ));
    };
    Ok(endpoint.delegation_of(owner).await? == Some(authorization.address))
}

fn apply_gas(operation: &mut UserOperation, gas: UserOperationGasEstimate) {
    operation.call_gas_limit = gas.call_gas_limit;
    operation.verification_gas_limit = gas.verification_gas_limit;
    operation.pre_verification_gas = gas.pre_verification_gas;
    operation.paymaster_verification_gas_limit = gas.paymaster_verification_gas_limit;
    operation.paymaster_post_op_gas_limit = gas.paymaster_post_op_gas_limit;
    operation.max_fee_per_gas = gas.max_fee_per_gas;
    operation.max_priority_fee_per_gas = gas.max_priority_fee_per_gas;
}

#[cfg(test)]
mod tests {
    use alloy_signer::k256::ecdsa::SigningKey;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{signer::simple::SimpleSigner, test_support::mocked_provider};

    #[tokio::test]
    async fn a_transaction_is_not_signed_by_anyone_but_its_sender() {
        let key = SigningKey::from_slice(&[1; 32]).unwrap();
        let prepared = PreparedTransaction {
            request: TransactionRequest::default().from(Address::repeat_byte(0x11)),
        };
        let refused = submit_prepared(
            mocked_provider(&Asserter::new()).as_ref(),
            Arc::new(SimpleSigner::from_key(key)),
            prepared,
        )
        .await;
        assert!(matches!(refused, Err(BroadcastError::WrongSigner { .. })));
    }
}
