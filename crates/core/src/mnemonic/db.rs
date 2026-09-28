use super::MnemonicRecord;
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait MnemonicDb: Database {
    async fn get_mnemonics(&self) -> Result<Vec<MnemonicRecord>, MnemonicDatabaseError> {
        let Some(bytes) = self.get(b"mnemonics").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_mnemonics(
        &self,
        mnemonics: &[MnemonicRecord],
    ) -> Result<(), MnemonicDatabaseError> {
        self.put(b"mnemonics", &postcard::to_stdvec(mnemonics)?)
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MnemonicDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> MnemonicDb for D {}
