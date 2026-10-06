use zeroize::Zeroizing;

use super::MnemonicRecord;
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait MnemonicDb: Database {
    async fn get_mnemonic_indices(&self) -> Result<Vec<u32>, MnemonicDatabaseError> {
        let Some(bytes) = self.get(b"mnemonics").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_mnemonic_indices(&self, indices: &[u32]) -> Result<(), MnemonicDatabaseError> {
        self.put(b"mnemonics", &postcard::to_stdvec(indices)?)
            .await?;
        Ok(())
    }

    async fn get_mnemonic(
        &self,
        index: u32,
    ) -> Result<Option<MnemonicRecord>, MnemonicDatabaseError> {
        let Some(bytes) = self.get(format!("mnemonic:{index}").as_bytes()).await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn put_mnemonic(&self, record: &MnemonicRecord) -> Result<(), MnemonicDatabaseError> {
        let bytes = Zeroizing::new(postcard::to_stdvec(record)?);
        self.put(format!("mnemonic:{}", record.index).as_bytes(), &bytes)
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
