use alloy_signer::k256::ecdsa::SigningKey;
use ethereum_desktop_wallet_core::database::{Database, DatabaseError};

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
        self.put(b"pk", &signing_key.to_bytes()).await?;
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
}

impl<D: Database + ?Sized> SimpleVaultDb for D {}
