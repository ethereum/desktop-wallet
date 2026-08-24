use alloy_primitives::Address;
use alloy_signer::k256::ecdsa::SigningKey;
use zeroize::Zeroizing;

use crate::database::{Database, DatabaseError};

pub trait SimpleVaultDb: Database {
    async fn get_signing_key(&self) -> Result<SigningKey, SimpleVaultDatabaseError> {
        let Some(pk) = self.get(b"pk").await? else {
            return Err(SimpleVaultDatabaseError::MissingPrivateKey);
        };

        let signing_key = SigningKey::from_slice(&pk)?;
        Ok(signing_key)
    }

    async fn put_signing_key(
        &self,
        signing_key: &SigningKey,
    ) -> Result<(), SimpleVaultDatabaseError> {
        let bytes = Zeroizing::new(signing_key.to_bytes().to_vec());
        self.put(b"pk", &bytes).await?;
        Ok(())
    }

    async fn get_implementation(&self) -> Result<Address, SimpleVaultDatabaseError> {
        let Some(bytes) = self.get(b"implementation").await? else {
            return Err(SimpleVaultDatabaseError::MissingImplementation);
        };
        Address::try_from(bytes.as_slice())
            .map_err(|_| SimpleVaultDatabaseError::InvalidImplementation)
    }

    async fn put_implementation(
        &self,
        implementation: &Address,
    ) -> Result<(), SimpleVaultDatabaseError> {
        self.put(b"implementation", implementation.as_slice())
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleVaultDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("invalid private key: {0}")]
    InvalidPrivateKey(#[from] alloy_signer::k256::ecdsa::Error),
    #[error("missing private key")]
    MissingPrivateKey,
    #[error("missing implementation address")]
    MissingImplementation,
    #[error("invalid implementation address")]
    InvalidImplementation,
}

impl<D: Database + ?Sized> SimpleVaultDb for D {}
