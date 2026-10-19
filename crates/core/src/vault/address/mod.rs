use std::sync::Arc;

use alloy_primitives::{Address, U256};

use crate::{
    asset::{self, AssetError},
    network::endpoint::NetworkEndpointError,
    prelude::*,
    vault::{
        Balance, Visibility,
        address::db::{AddressVaultDatabaseError, AddressVaultDb},
    },
};

pub mod db;

pub const ADDRESS_VAULT_TAG: &str = "address";

/// A [`Vault`] for one plain address: it reads balances and holds no key.
pub struct AddressVault {
    address: Address,
    provider: Arc<dyn NetworkEndpoint>,
}

#[derive(Debug, thiserror::Error)]
pub enum AddressVaultError {
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("database error: {0}")]
    Database(#[from] AddressVaultDatabaseError),
    #[error("network error: {0}")]
    Network(#[from] NetworkEndpointError),
}

inventory::submit! {
    Factory::new(ADDRESS_VAULT_TAG, |ctx: BuildContext| {
        Box::pin(async move {
            AddressVault::from_context(ctx).await.map_err(|e| FactoryError::Other(Box::new(e)))
        })
    })
}

impl AddressVault {
    async fn from_context(ctx: BuildContext) -> Result<Box<dyn Vault>, AddressVaultError> {
        let derived = ctx.db.get_derived_address().await?;
        Ok(Box::new(Self {
            address: derived.address,
            provider: ctx.provider,
        }))
    }

    async fn amount(&self, asset: &AssetId) -> Result<U256, AddressVaultError> {
        Ok(match asset {
            AssetId::Native => self.provider.balance(self.address).await?,
            AssetId::Erc20(token) => {
                asset::erc20_balance(self.provider.as_ref(), *token, self.address).await?
            }
        })
    }
}

#[async_trait::async_trait]
impl Vault for AddressVault {
    fn tag(&self) -> &'static str {
        ADDRESS_VAULT_TAG
    }

    fn id(&self) -> VaultId {
        VaultId::Address(self.address)
    }

    fn visibility(&self) -> Visibility {
        Visibility::Public
    }

    async fn balance(&self, asset: &AssetId) -> Result<Balance, VaultError> {
        Ok(Balance::spendable(self.amount(asset).await?))
    }
}

impl From<AddressVaultError> for VaultError {
    fn from(err: AddressVaultError) -> Self {
        VaultError::Other(Box::new(err))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Bytes, address};
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::test_support::mocked_provider;

    const TOKEN: Address = address!("0x1111111111111111111111111111111111111111");

    fn vault(asserter: &Asserter) -> AddressVault {
        AddressVault {
            address: Address::repeat_byte(0xaa),
            provider: mocked_provider(asserter),
        }
    }

    #[tokio::test]
    async fn a_native_balance_is_spendable() {
        let asserter = Asserter::new();
        asserter.push_success(&U256::from(7));

        let balance = vault(&asserter).balance(&AssetId::Native).await.unwrap();

        assert_eq!(balance, Balance::spendable(U256::from(7)));
    }

    #[tokio::test]
    async fn an_erc20_balance_is_read_by_a_contract_call() {
        let asserter = Asserter::new();
        // Too long for a quantity, so only a contract call's reply decodes it.
        asserter.push_success(&Bytes::from(
            [U256::from(9).to_be_bytes::<32>(), [0; 32]].concat(),
        ));

        let balance = vault(&asserter)
            .balance(&AssetId::Erc20(TOKEN))
            .await
            .unwrap();

        assert_eq!(balance, Balance::spendable(U256::from(9)));
    }
}
