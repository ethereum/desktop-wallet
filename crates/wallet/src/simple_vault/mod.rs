use std::sync::Arc;

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{SignedAuthorization, TransactionRequest};
use alloy_sol_types::SolCall;
use edw_core::{
    asset::AssetId,
    call::Call,
    database::Database,
    factory::{BuildContext, Factory, FactoryError, try_build_signer},
    signer::Signer,
    vault::{Vault, VaultError, VaultId},
};

use crate::{
    simple_delegate::{
        SIMPLE_DELEGATE_ADDRESS, SimpleDelegate, SimpleDelegateError, signer_address,
    },
    simple_signer::{
        db::{SimpleSignerDatabaseError, SimpleSignerDb},
        persist_and_rebuild,
    },
    simple_vault::db::{SimpleVaultDatabaseError, SimpleVaultDb},
};

pub(crate) mod db;

/// `SimpleVault` is a basic [`Vault`] implementation that uses a signer-based wallet
/// to store and transfer assets through its address. It uses the`SimpleDelegate`
/// contract to allow the signer to authorize vault transactions for withdrawals.
pub struct SimpleVault {
    delegate: SimpleDelegate,
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
    #[error("signer database error: {0}")]
    SignerDatabase(#[from] SimpleSignerDatabaseError),
    #[error("factory error: {0}")]
    Factory(#[from] FactoryError),
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

const SIMPLE_VAULT_TAG: &str = "simple-vault";

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
        signer: Arc<dyn Signer>,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleVaultError> {
        Self::new_with_implementation(signer, SIMPLE_DELEGATE_ADDRESS, provider, db).await
    }

    /// Returns a 7702 delegation authorization for the `SimpleVault` contract. The caller
    /// submits it; this does not check whether the address is already delegated.
    ///
    /// # Errors
    /// Returns an error if an RPC call fails or if the authorization cannot be signed.
    pub async fn authorization(
        signer: &dyn Signer,
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
        let provider = ctx.provider.clone();
        let db = ctx.db.clone();

        let tag = ctx.db.get_signer_tag().await?;
        let signer: Arc<dyn Signer> = Arc::from(try_build_signer(&tag, ctx.clone()).await?);
        let implementation = db.get_implementation().await?;
        let vault =
            SimpleVault::new_with_implementation(signer, implementation, provider, db).await?;
        Ok(Box::new(vault))
    }

    /// Creates a new `SimpleVault` instance.
    ///
    /// `signer`'s [`Signer::tag`] is recorded, then the signer is rebuilt from `db`, so
    /// [`SimpleVault::from_context`] can recover it. This happens before the delegate is
    /// constructed.
    ///
    /// # Errors
    /// Returns an error if the signer cannot be rebuilt from `db`, if the signer's code is
    /// not delegated to the implementation address, or if there is an RPC error.
    pub async fn new_with_implementation(
        signer: Arc<dyn Signer>,
        implementation: Address,
        provider: Arc<dyn Provider>,
        db: Arc<dyn Database>,
    ) -> Result<Self, SimpleVaultError> {
        persist_and_rebuild(
            signer.as_ref(),
            BuildContext::new(provider.clone(), db.clone()),
        )
        .await?;
        let delegate =
            SimpleDelegate::new_with_implementation(signer, implementation, provider.clone())
                .await?;

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
        signer: &dyn Signer,
        implementation: Address,
        provider: &dyn Provider,
    ) -> Result<SignedAuthorization, SimpleVaultError> {
        let nonce = provider
            .get_transaction_count(signer_address(signer))
            .await?;
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

#[cfg(test)]
mod tests {
    use alloy_primitives::U64;
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
        // The delegate checks the delegation, then reads the chain id for its EIP-712 domain.
        asserter.push_success(&delegation_designator_code(SIMPLE_DELEGATE_ADDRESS));
        asserter.push_success(&U64::from(1));

        let vault = SimpleVault::from_context(BuildContext::new(mocked_provider(&asserter), db))
            .await
            .expect("rebuild from the stored tag");

        assert_eq!(
            vault.id(),
            VaultId::Address(address),
            "the rebuilt vault must belong to the key the stored tag names",
        );
    }

    #[tokio::test]
    async fn from_context_fails_before_the_chain_when_no_tag_is_stored() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let asserter = Asserter::new();
        asserter.push_success(&delegation_designator_code(SIMPLE_DELEGATE_ADDRESS));

        let result =
            SimpleVault::from_context(BuildContext::new(mocked_provider(&asserter), db)).await;

        let Err(error) = result else {
            panic!("there is nothing to rebuild from");
        };
        assert!(
            matches!(
                error,
                SimpleVaultError::SignerDatabase(SimpleSignerDatabaseError::MissingSignerTag)
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
            SimpleVault::from_context(BuildContext::new(mocked_provider(&asserter), db)).await;

        let Err(error) = result else {
            panic!("the tag names a signer whose key was never stored");
        };
        assert!(
            matches!(error, SimpleVaultError::Factory(_)),
            "expected the factory to refuse, got {error}",
        );
        assert_eq!(
            asserter.read_q().len(),
            1,
            "a build whose signer cannot be rebuilt must not reach the chain",
        );
    }
}
