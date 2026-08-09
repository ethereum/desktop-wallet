use alloy::{
    primitives::{Address, Bytes, U256},
    providers::Provider,
    rpc::types::{SignedAuthorization, TransactionRequest},
    signers::local::PrivateKeySigner,
    sol_types::SolCall,
};

use crate::{
    call::Call,
    profile::{
        AssetId, VaultId,
        vault::{Vault, VaultError},
    },
    simple_delegate::{SIMPLE_DELEGATE_ADDRESS, SimpleDelegate, SimpleDelegateError},
};

pub struct SimpleVault<P: Provider> {
    delegate: SimpleDelegate<P>,
    provider: P,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleVaultError {
    #[error(transparent)]
    Delegate(#[from] SimpleDelegateError),
    #[error("address not authorized")]
    NotAuthorized,
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),
    #[error("sol error: {0}")]
    Sol(#[from] alloy::sol_types::Error),
}

mod sol {
    use alloy::sol;

    sol!(
        contract Erc20 {
            // ERC20
            function balanceOf(address) external view returns (uint256);
            function transfer(address to, uint256 amount) external returns (bool);
        }
    );
}

impl<P: Provider + Clone> SimpleVault<P> {
    /// Creates a new `SimpleVault` instance with the given signer and provider.
    ///
    /// # Errors
    /// Returns an error if the signer's address is not delegated to the `SimpleVault`
    /// implementation contract.
    pub async fn new(signer: PrivateKeySigner, provider: P) -> Result<Self, SimpleVaultError> {
        Self::new_with_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider).await
    }

    /// Returns a 7702 delegation authorization for the `SimpleVault` contract. If the
    /// address is already authorized, returns `None`.
    ///
    /// # Errors
    /// Returns an error if an RPC call fails or if the authorization cannot be signed.
    pub async fn authorization(
        signer: &PrivateKeySigner,
        provider: &P,
    ) -> Result<SignedAuthorization, SimpleVaultError> {
        Self::authorize_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider).await
    }

    /// Creates a new `SimpleVault` instance.
    ///
    /// # Errors
    /// Returns an error if the signer's code is not delegated to the implementation
    /// address or if there is an RPC error.
    pub async fn new_with_implementation(
        signer: PrivateKeySigner,
        implementation: Address,
        provider: P,
    ) -> Result<Self, SimpleVaultError> {
        let delegate =
            SimpleDelegate::new_with_implementation(signer, implementation, provider.clone())
                .await?;
        Ok(Self { delegate, provider })
    }

    /// Submits the 7702 authorization transaction on-chain if the delegate is not already authorized
    /// for the signer's address.
    ///
    /// # Errors
    /// Returns an error if there is an RPC error or if the authorization cannot be signed.
    pub async fn authorize_implementation(
        signer: &PrivateKeySigner,
        implementation: Address,
        provider: &P,
    ) -> Result<SignedAuthorization, SimpleVaultError> {
        let nonce = provider.get_transaction_count(signer.address()).await?;
        let auth =
            SimpleDelegate::authorize_implementation(signer, nonce, provider, implementation)
                .await?;
        Ok(auth)
    }
}

#[async_trait::async_trait]
impl<P: Provider> Vault for SimpleVault<P> {
    fn id(&self) -> VaultId {
        VaultId::Address(self.address())
    }

    async fn balance(&self, asset: &AssetId) -> Result<U256, VaultError> {
        match asset {
            AssetId::Native => Ok(self.balance_native().await?),
            AssetId::Erc20(token) => Ok(self.balance_erc20(*token).await?),
        }
    }

    async fn deposit(
        &self,
        _from: Address,
        asset: &AssetId,
        amount: U256,
    ) -> Result<Vec<Call>, VaultError> {
        let calls = match asset {
            AssetId::Native => self.deposit_native(amount),
            AssetId::Erc20(token) => self.deposit_erc20(*token, amount),
        };
        Ok(calls)
    }

    async fn withdraw(
        &self,
        to: &VaultId,
        asset: &AssetId,
        amount: U256,
    ) -> Result<Vec<Call>, VaultError> {
        #[allow(irrefutable_let_patterns)]
        let VaultId::Address(address) = to else {
            return Err(VaultError::UnsupportedVaultId(to.clone()));
        };

        let calls = match asset {
            AssetId::Native => self.withdraw_native(*address, amount).await?,
            AssetId::Erc20(token) => self.withdraw_erc20(*address, *token, amount).await?,
        };
        Ok(calls)
    }
}

impl<P: Provider> SimpleVault<P> {
    fn address(&self) -> Address {
        self.delegate.address()
    }

    async fn balance_native(&self) -> Result<U256, SimpleVaultError> {
        let balance = self.provider.get_balance(self.address()).await?;
        Ok(balance)
    }

    async fn balance_erc20(&self, token: Address) -> Result<U256, SimpleVaultError> {
        let call = sol::Erc20::balanceOfCall::new((self.address(),));
        let data = self
            .provider
            .call(
                TransactionRequest::default()
                    .to(token)
                    .input(call.abi_encode().into()),
            )
            .await?;

        let balance = sol::Erc20::balanceOfCall::abi_decode_returns(&data)?;
        Ok(balance)
    }

    fn deposit_native(&self, amount: U256) -> Vec<Call> {
        vec![Call::new(self.address(), Bytes::new(), amount)]
    }

    fn deposit_erc20(&self, token: Address, amount: U256) -> Vec<Call> {
        let data = sol::Erc20::transferCall::new((self.address(), amount))
            .abi_encode()
            .into();
        vec![Call::new(token, data, U256::ZERO)]
    }

    async fn withdraw_native(
        &self,
        to: Address,
        amount: U256,
    ) -> Result<Vec<Call>, SimpleVaultError> {
        let call = Call::new(to, Bytes::new(), amount);
        let c = self.delegate.batch_calls(&[call]).await?;
        Ok(vec![c])
    }

    async fn withdraw_erc20(
        &self,
        to: Address,
        token: Address,
        amount: U256,
    ) -> Result<Vec<Call>, SimpleVaultError> {
        let data = sol::Erc20::transferCall::new((to, amount))
            .abi_encode()
            .into();

        let call = Call::new(token, data, U256::ZERO);
        let c = self.delegate.batch_calls(&[call]).await?;
        Ok(vec![c])
    }
}

impl From<SimpleVaultError> for VaultError {
    fn from(err: SimpleVaultError) -> Self {
        VaultError::Inner(Box::new(err))
    }
}
