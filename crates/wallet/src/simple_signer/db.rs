use alloy_signer::k256::ecdsa::SigningKey;
use edw_core::database::{Database, DatabaseError};
use zeroize::Zeroizing;

pub trait SimpleSignerDb: Database {
    async fn get_signing_key(&self) -> Result<SigningKey, SimpleSignerDatabaseError> {
        let Some(pk) = self.get(b"pk").await? else {
            return Err(SimpleSignerDatabaseError::MissingPrivateKey);
        };

        let signing_key = SigningKey::from_slice(&pk)?;
        Ok(signing_key)
    }

    async fn put_signing_key(
        &self,
        signing_key: &SigningKey,
    ) -> Result<(), SimpleSignerDatabaseError> {
        let bytes = Zeroizing::new(signing_key.to_bytes().to_vec());
        self.put(b"pk", &bytes).await?;
        Ok(())
    }

    /// The factory tag of the signer stored here, so an owner rebuilds the kind it was given
    /// rather than assuming one.
    async fn get_signer_tag(&self) -> Result<String, SimpleSignerDatabaseError> {
        let Some(bytes) = self.get(b"signer_tag").await? else {
            return Err(SimpleSignerDatabaseError::MissingSignerTag);
        };
        String::from_utf8(bytes.to_vec()).map_err(|_| SimpleSignerDatabaseError::InvalidSignerTag)
    }

    async fn put_signer_tag(&self, tag: &str) -> Result<(), SimpleSignerDatabaseError> {
        self.put(b"signer_tag", tag.as_bytes()).await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleSignerDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("invalid private key: {0}")]
    InvalidPrivateKey(#[from] alloy_signer::k256::ecdsa::Error),
    #[error("missing private key")]
    MissingPrivateKey,
    #[error("missing signer tag")]
    MissingSignerTag,
    #[error("invalid signer tag")]
    InvalidSignerTag,
}

impl<D: Database + ?Sized> SimpleSignerDb for D {}
