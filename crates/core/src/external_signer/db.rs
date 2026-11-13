use super::ExternalSignerRecord;
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait ExternalSignerDb: Database {
    async fn get_external_signers(
        &self,
    ) -> Result<Vec<ExternalSignerRecord>, ExternalSignerDatabaseError> {
        let Some(bytes) = self.get(b"externalSigners").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_external_signers(
        &self,
        signers: &[ExternalSignerRecord],
    ) -> Result<(), ExternalSignerDatabaseError> {
        self.put(b"externalSigners", &postcard::to_stdvec(signers)?)
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExternalSignerDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> ExternalSignerDb for D {}
