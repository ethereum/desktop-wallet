use alloy::{
    eips::eip7702::constants::EIP7702_DELEGATION_DESIGNATOR,
    primitives::{Address, Bytes, U256, address},
    providers::Provider,
    rpc::types::{Authorization, SignedAuthorization, TransactionRequest},
    signers::{Signer, local::PrivateKeySigner},
    sol_types::{SolCall, eip712_domain},
};

use crate::call::Call;

pub struct SimpleDelegate<P: Provider> {
    chain_id: u64,
    signer: PrivateKeySigner,
    provider: P,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleDelegateError {
    #[error("address not authorized")]
    NotAuthorized,
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),
    #[error("signer error: {0}")]
    Signer(#[from] alloy::signers::Error),
    #[error("sol error: {0}")]
    Sol(#[from] alloy::sol_types::Error),
}

mod sol {
    use alloy::sol;

    sol!(
        struct Call {
            address target;
            uint256 value;
            bytes data;
        }

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

impl<P: Provider> SimpleDelegate<P> {
    /// Creates a new `SimpleDelegate` instance.
    ///
    /// # Errors
    /// Returns an error if the signer is not authorized to act as a delegate for the given address.
    pub async fn new_with_delegate(
        signer: PrivateKeySigner,
        provider: P,
        delegate: Address,
    ) -> Result<Self, SimpleDelegateError> {
        if !Self::authorized(signer.address(), delegate, &provider).await? {
            return Err(SimpleDelegateError::NotAuthorized);
        }

        let chain_id = provider.get_chain_id().await?;
        Ok(Self {
            chain_id,
            signer,
            provider,
        })
    }

    /// Returns a signed 7702 authorization for the given delegate address.
    ///
    /// # Errors
    /// Returns an error if an RPC error occurs or if the signer fails to sign
    /// the authorization.
    pub async fn authorize_delegate(
        signer: &PrivateKeySigner,
        nonce: u64,
        provider: &P,
        delegate: Address,
    ) -> Result<SignedAuthorization, SimpleDelegateError> {
        let chain_id = provider.get_chain_id().await?;

        let authorization = Authorization {
            chain_id: U256::from(chain_id),
            address: delegate,
            nonce,
        };

        let signature = signer.sign_hash(&authorization.signature_hash()).await?;
        Ok(authorization.into_signed(signature))
    }

    pub async fn authorized(
        address: Address,
        expected: Address,
        provider: &P,
    ) -> Result<bool, SimpleDelegateError> {
        let code = provider.get_code_at(address).await?;
        let expected = delegation_code(expected);
        Ok(code == expected)
    }

    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// Signs a batch of calls to be executed by the `SimpleVault` contract.
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

        let signature = self.signer.sign_typed_data(&batch, &self.domain()).await?;
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

    fn domain(&self) -> alloy::dyn_abi::Eip712Domain {
        eip712_domain! {
            name: "SimpleDelegate",
            version: "1",
            chain_id: self.chain_id,
            verifying_contract: self.address(),
        }
    }
}

fn delegation_code(delegate: Address) -> Bytes {
    let mut code = Vec::with_capacity(23);
    code.extend_from_slice(&EIP7702_DELEGATION_DESIGNATOR);
    code.extend_from_slice(delegate.as_slice());
    Bytes::from(code)
}
