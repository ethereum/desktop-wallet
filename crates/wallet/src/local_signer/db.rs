use alloy_signer::k256::ecdsa::SigningKey;
use edw_core::database::{Database, DatabaseError};
use zeroize::Zeroizing;

pub trait LocalSignerDb: Database {
    async fn get_signing_key(&self) -> Result<SigningKey, LocalSignerDatabaseError> {
        let Some(pk) = self.get(b"pk").await? else {
            return Err(LocalSignerDatabaseError::MissingPrivateKey);
        };

        let signing_key = SigningKey::from_slice(&pk)?;
        Ok(signing_key)
    }

    async fn put_signing_key(
        &self,
        signing_key: &SigningKey,
    ) -> Result<(), LocalSignerDatabaseError> {
        let bytes = Zeroizing::new(signing_key.to_bytes().to_vec());
        self.put(b"pk", &bytes).await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LocalSignerDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("invalid private key: {0}")]
    InvalidPrivateKey(#[from] alloy_signer::k256::ecdsa::Error),
    #[error("missing private key")]
    MissingPrivateKey,
}

impl<D: Database + ?Sized> LocalSignerDb for D {}
