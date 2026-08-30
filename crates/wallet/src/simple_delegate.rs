use std::sync::Arc;

use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::{
    Authorization, SignedAuthorization, constants::EIP7702_DELEGATION_DESIGNATOR,
};
use alloy_primitives::{Address, Bytes, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types_eth::TransactionRequest;
use alloy_sol_types::{Eip712Domain, SolCall, eip712_domain};
use edw_core::{
    call::Call,
    signer::{Signer, SignerError, SignerId},
};

/// `SimpleDelegate` is a 7702-compatible delegate contract loosely based on Safe's
/// [`SafeLite`](https://github.com/5afe/safe-eip7702/blob/main/safe-eip7702-contracts/contracts/experimental/SafeLite.sol)
/// contract. An address can authorize the `SimpleDelegate` contract with a 7702
/// authorization, then execute signed batches of calls. This is used for atomic
/// execution of multiple calls and gasless execution for the signer.
pub struct SimpleDelegate {
    chain_id: u64,
    signer: Arc<dyn Signer>,
    provider: Arc<dyn Provider>,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleDelegateError {
    #[error("address not authorized")]
    NotAuthorized,
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy_transport::RpcError<alloy_transport::TransportErrorKind>),
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("sol error: {0}")]
    Sol(#[from] alloy_sol_types::Error),
}

mod sol {
    use alloy_sol_types::sol;

    sol!(
        #[derive(serde::Serialize)]
        struct Call {
            address target;
            uint256 value;
            bytes data;
        }

        #[derive(serde::Serialize)]
        struct ExecuteBatch {
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

impl SimpleDelegate {
    /// Creates a new `SimpleDelegate` instance.
    ///
    /// # Errors
    /// Returns an error if the signer's code is not delegated to the implementation
    /// address or if there is an RPC error.
    pub async fn new_with_implementation(
        signer: Arc<dyn Signer>,
        implementation: Address,
        provider: Arc<dyn Provider>,
    ) -> Result<Self, SimpleDelegateError> {
        if !is_delegated(
            signer_address(signer.as_ref()),
            implementation,
            provider.as_ref(),
        )
        .await?
        {
            return Err(SimpleDelegateError::NotAuthorized);
        }

        let chain_id = provider.get_chain_id().await?;
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
        signer: &dyn Signer,
        nonce: u64,
        provider: &dyn Provider,
        implementation: Address,
    ) -> Result<SignedAuthorization, SimpleDelegateError> {
        let chain_id = provider.get_chain_id().await?;

        let authorization = Authorization {
            chain_id: U256::from(chain_id),
            address: implementation,
            nonce,
        };

        Ok(signer.sign_authorization(&authorization).await?)
    }

    #[must_use]
    pub fn address(&self) -> Address {
        signer_address(self.signer.as_ref())
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

        let batch = sol::ExecuteBatch {
            calls: calls.clone(),
            nonce: self.nonce().await?,
        };

        let typed_data = TypedData::from_struct(&batch, Some(self.domain()));
        let signature = self.signer.sign_typed_data(&typed_data).await?;
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

/// A [`SignerId`] variant without an address would need a different delegate, since 7702
/// delegation is a property of an EOA.
pub(crate) fn signer_address(signer: &dyn Signer) -> Address {
    match signer.id() {
        SignerId::Address(address) => address,
    }
}

/// Returns whether the given address is delegated to act as the implementation
/// for the given delegator.
pub async fn is_delegated(
    delegator: Address,
    implementation: Address,
    provider: &dyn Provider,
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
