use std::sync::Arc;

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{SignedAuthorization, TransactionRequest};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
use ethereum_desktop_wallet_core::{
    asset::AssetId,
    call::Call,
    database::Database,
    factory::{BuildContext, Factory, FactoryError},
    vault::{Vault, VaultError, VaultId},
};

use crate::{
    simple_delegate::{SIMPLE_DELEGATE_ADDRESS, SimpleDelegate, SimpleDelegateError},
    simple_vault::db::{SimpleVaultDatabaseError, SimpleVaultDb},
};

mod db;

/// `SimpleVault` is a basic [`Vault`] implementation that uses a signer-based wallet
/// to store and transfer assets through its address. It uses the`SimpleDelegate`
/// contract to allow the signer to authorize vault transactions for withdrawals.
pub struct SimpleVault {
    delegate: SimpleDelegate<PrivateKeySigner>,
    provider: Arc<dyn Provider>,
    #[allow(unused)]
    db: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleVaultError {
    #[error(transparent)]
    Delegate(#[from] SimpleDelegateError),
    #[error("database error: {0}")]
    Database(#[from] SimpleVaultDatabaseError),
    #[error("address not authorized")]
    NotAuthorized,
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy_transport::RpcError<alloy_transport::TransportErrorKind>),
    #[error("sol error: {0}")]
    Sol(#[from] alloy_sol_types::Error),
}

mod sol {
    use alloy_sol_types::sol;

    sol!(
        contract Erc20 {
            // ERC20
            function balanceOf(address) external view returns (uint256);
            function transfer(address to, uint256 amount) external returns (bool);
        }
    );
}

const SIMPLE_VAULT_TAG: &'static str = "simple-vault";

inventory::submit! {
    Factory::new(SIMPLE_VAULT_TAG, |ctx: BuildContext| {
        Box::pin(async move {
            SimpleVault::from_context(ctx).await.map_err(|e| FactoryError::Other(Box::new(e)))
        })
    })
}

impl SimpleVault {
    /// Creates a new `SimpleVault` instance with the given signer and provider.
    ///
    /// # Errors
    /// Returns an error if the signer's address is not delegated to the `SimpleVault`
    /// implementation contract.
    pub async fn new(
        signer: PrivateKeySigner,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleVaultError> {
        Self::new_with_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider, db).await
    }

    /// Returns a 7702 delegation authorization for the `SimpleVault` contract. If the
    /// address is already authorized, returns `None`.
    ///
    /// # Errors
    /// Returns an error if an RPC call fails or if the authorization cannot be signed.
    pub async fn authorization(
        signer: &PrivateKeySigner,
        provider: &dyn Provider,
    ) -> Result<SignedAuthorization, SimpleVaultError> {
        Self::authorize_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider).await
    }

    /// Creates a new `SimpleVault` instance from the given context.
    ///
    /// # Errors
    /// Errors if the signer cannot be retrieved from the database or if the `SimpleVault` cannot
    /// be created (see [`SimpleVault::new`]).
    pub async fn from_context(ctx: BuildContext) -> Result<Box<dyn Vault>, SimpleVaultError> {
        let provider = ctx.provider;
        let db = ctx.db;

        let signing_key = db.get_signing_key().await?;
        let signer = PrivateKeySigner::from_signing_key(signing_key);
        let implementation = db.get_implementation().await?;
        let vault =
            SimpleVault::new_with_implementation(signer, implementation, provider, db).await?;
        Ok(Box::new(vault))
    }

    /// Creates a new `SimpleVault` instance.
    ///
    /// # Errors
    /// Returns an error if the signer's code is not delegated to the implementation
    /// address or if there is an RPC error.
    pub async fn new_with_implementation(
        signer: PrivateKeySigner,
        implementation: Address,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleVaultError> {
        let delegate =
            SimpleDelegate::new_with_implementation(signer, implementation, provider.clone())
                .await?;

        db.put_signing_key(delegate.signer().credential()).await?;
        db.put_implementation(&implementation).await?;
        Ok(Self {
            delegate,
            provider,
            db,
        })
    }

    /// Submits the 7702 authorization transaction on-chain if the delegate is not already authorized
    /// for the signer's address.
    ///
    /// # Errors
    /// Returns an error if there is an RPC error or if the authorization cannot be signed.
    pub async fn authorize_implementation(
        signer: &PrivateKeySigner,
        implementation: Address,
        provider: &dyn Provider,
    ) -> Result<SignedAuthorization, SimpleVaultError> {
        let nonce = provider.get_transaction_count(signer.address()).await?;
        let auth =
            SimpleDelegate::authorize_implementation(signer, nonce, provider, implementation)
                .await?;
        Ok(auth)
    }
}

#[async_trait::async_trait]
impl Vault for SimpleVault {
    fn tag(&self) -> &'static str {
        SIMPLE_VAULT_TAG
    }

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

impl SimpleVault {
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

    /// Withdraws native tokens from the vault to the specific address.
    ///
    /// The vault's signer authorizes the withdrawal by returning a signed
    /// `executeBatch` call to the `SimpleDelegate` contract.
    async fn withdraw_native(
        &self,
        to: Address,
        amount: U256,
    ) -> Result<Vec<Call>, SimpleVaultError> {
        let call = Call::new(to, Bytes::new(), amount);
        let c = self.delegate.batch_calls(&[call]).await?;
        Ok(vec![c])
    }

    /// Withdraws ERC20 tokens from the vault to the specific address.
    ///
    /// The vault's signer authorizes the withdrawal by returning a signed
    /// `executeBatch` call to the `SimpleDelegate` contract.
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
        VaultError::Other(Box::new(err))
    }
}
