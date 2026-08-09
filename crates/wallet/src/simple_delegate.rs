use std::sync::Arc;

use alloy_eips::eip7702::{
    Authorization, SignedAuthorization, constants::EIP7702_DELEGATION_DESIGNATOR,
};
use alloy_primitives::{Address, Bytes, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types_eth::TransactionRequest;
use alloy_signer::Signer;
use alloy_sol_types::{Eip712Domain, SolCall, SolStruct, eip712_domain};
use ethereum_desktop_wallet_core::call::Call;

/// `SimpleDelegate` is a 7702-compatible delegate contract loosely based on Safe's
/// [`SafeLite`](https://github.com/5afe/safe-eip7702/blob/main/safe-eip7702-contracts/contracts/experimental/SafeLite.sol)
/// contract. An address can authorize the `SimpleDelegate` contract with a 7702
/// authorization, then execute signed batches of calls. This is used for atomic
/// execution of multiple calls and gasless execution for the signer.
pub struct SimpleDelegate<P: Provider> {
    chain_id: u64,
    signer: Arc<dyn Signer + Send + Sync>,
    provider: P,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleDelegateError {
    #[error("address not authorized")]
    NotAuthorized,
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy_transport::RpcError<alloy_transport::TransportErrorKind>),
    #[error("signer error: {0}")]
    Signer(#[from] alloy_signer::Error),
    #[error("sol error: {0}")]
    Sol(#[from] alloy_sol_types::Error),
}

mod sol {
    use alloy_sol_types::sol;

    sol!(
        struct Call {
            address target;
            uint256 value;
            bytes data;
        }

        struct Batch {
            Call[] calls;
            uint256 nonce;
        }

        contract SimpleDelegate {
            function nonce() external view returns (uint256);
            function executeBatch(
                Call[] calldata calls,
                uint8 v,
                bytes32 r,
                bytes32 s
            ) external;
        }
    );
}

pub const SIMPLE_DELEGATE_ADDRESS: Address = address!("0xACAe14c5d84EA4a1ddb84bFbDc1a62796677ACcA");

impl<P: Provider> SimpleDelegate<P> {
    /// Creates a new `SimpleDelegate` instance.
    ///
    /// # Errors
    /// Returns an error if the signer's code is not delegated to the implementation
    /// address or if there is an RPC error.
    pub async fn new_with_implementation(
        signer: impl Signer + Send + Sync + 'static,
        implementation: Address,
        provider: P,
    ) -> Result<Self, SimpleDelegateError> {
        if !is_delegated(signer.address(), implementation, &provider).await? {
            return Err(SimpleDelegateError::NotAuthorized);
        }

        let chain_id = provider.get_chain_id().await?;
        let signer = Arc::new(signer);
        Ok(Self {
            chain_id,
            signer,
            provider,
        })
    }

    /// Returns a signed 7702 authorization for the given implementation address.
    ///
    /// # Errors
    /// Returns an error if an RPC error occurs or if the signer fails to sign
    /// the authorization.
    pub async fn authorize_implementation(
        signer: &impl Signer,
        nonce: u64,
        provider: &P,
        implementation: Address,
    ) -> Result<SignedAuthorization, SimpleDelegateError> {
        let chain_id = provider.get_chain_id().await?;

        let authorization = Authorization {
            chain_id: U256::from(chain_id),
            address: implementation,
            nonce,
        };

        let signature = signer.sign_hash(&authorization.signature_hash()).await?;
        Ok(authorization.into_signed(signature))
    }

    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// Signs a batch of calls to be executed by the `SimpleVault` contract. Returns
    /// a [`Call`] that can be submitted to this address to execute the batch.
    pub async fn batch_calls(&self, calls: &[Call]) -> Result<Call, SimpleDelegateError> {
        let calls: Vec<_> = calls
            .iter()
            .map(|call| sol::Call {
                target: call.target,
                value: call.value,
                data: call.data.clone(),
            })
            .collect();

        let batch = sol::Batch {
            calls: calls.clone(),
            nonce: self.nonce().await?,
        };

        let digest = batch.eip712_signing_hash(&self.domain());
        let signature = self.signer.sign_hash(&digest).await?;
        let data = sol::SimpleDelegate::executeBatchCall {
            calls,
            v: u8::from(signature.v()) + 27,
            r: signature.r().into(),
            s: signature.s().into(),
        }
        .abi_encode()
        .into();

        Ok(Call::new(self.address(), data, U256::ZERO))
    }

    async fn nonce(&self) -> Result<U256, SimpleDelegateError> {
        let call = sol::SimpleDelegate::nonceCall::new(());
        let data = self
            .provider
            .call(
                TransactionRequest::default()
                    .to(self.address())
                    .input(call.abi_encode().into()),
            )
            .await?;

        let nonce = sol::SimpleDelegate::nonceCall::abi_decode_returns(&data)?;
        Ok(nonce)
    }

    fn domain(&self) -> Eip712Domain {
        eip712_domain! {
            name: "SimpleDelegate",
            version: "1",
            chain_id: self.chain_id,
            verifying_contract: self.address(),
        }
    }
}

/// Returns whether the given address is delegated to act as the implementation
/// for the given delegator.
pub async fn is_delegated<P: Provider>(
    delegator: Address,
    implementation: Address,
    provider: &P,
) -> Result<bool, SimpleDelegateError> {
    let code = provider.get_code_at(delegator).await?;
    let expected = delegation_designator_code(implementation);
    Ok(code == expected)
}

/// Builds the EIP-7702 delegation designator bytecode that an EOA installs
/// on itself to delegate execution to `implementation`.
fn delegation_designator_code(implementation: Address) -> Bytes {
    let mut code = Vec::with_capacity(23);
    code.extend_from_slice(&EIP7702_DELEGATION_DESIGNATOR);
    code.extend_from_slice(implementation.as_slice());
    Bytes::from(code)
}
