use alloy_primitives::Address;
use alloy_signer::k256::ecdsa::SigningKey;
use edw_core::database::{Database, DatabaseError};

pub trait SimpleExecutorDb: Database {
    async fn get_signing_key(&self) -> Result<SigningKey, SimpleExecutorDatabaseError> {
        let Some(pk) = self.get(b"pk").await? else {
            return Err(SimpleExecutorDatabaseError::MissingPrivateKey);
        };

        let signing_key = SigningKey::from_slice(&pk)?;
        Ok(signing_key)
    }

    async fn put_signing_key(
        &self,
        signing_key: &SigningKey,
    ) -> Result<(), SimpleExecutorDatabaseError> {
        self.put(b"pk", &signing_key.to_bytes()).await?;
        Ok(())
    }

    async fn get_implementation(&self) -> Result<Address, SimpleExecutorDatabaseError> {
        let Some(bytes) = self.get(b"implementation").await? else {
            return Err(SimpleExecutorDatabaseError::MissingImplementation);
        };
        Address::try_from(bytes.as_slice())
            .map_err(|_| SimpleExecutorDatabaseError::InvalidImplementation)
    }

    async fn put_implementation(
        &self,
        implementation: &Address,
    ) -> Result<(), SimpleExecutorDatabaseError> {
        self.put(b"implementation", implementation.as_slice())
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleExecutorDatabaseError {
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

impl<D: Database + ?Sized> SimpleExecutorDb for D {}
