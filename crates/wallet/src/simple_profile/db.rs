use ethereum_desktop_wallet_core::database::{Database, DatabaseError};
use uuid::Uuid;

pub trait SimpleProfileDb: Database {
    async fn get_executor(&self) -> Result<Option<(Uuid, String)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"executor").await? else {
            return Ok(None);
        };
        let executor = postcard::from_bytes(&bytes)?;
        Ok(Some(executor))
    }

    async fn get_vaults(&self) -> Result<Vec<(Uuid, String)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"vaults").await? else {
            return Ok(vec![]);
        };
        let vaults = postcard::from_bytes(&bytes)?;
        Ok(vaults)
    }

    async fn put_executor(&self, executor: (Uuid, &str)) -> Result<(), SimpleProfileDatabaseError> {
        let bytes = postcard::to_stdvec(&executor)?;
        self.put(b"executor", &bytes).await?;
        Ok(())
    }

    async fn put_vaults(&self, vaults: &[(Uuid, &str)]) -> Result<(), SimpleProfileDatabaseError> {
        let bytes = postcard::to_stdvec(vaults)?;
        self.put(b"vaults", &bytes).await?;
        Ok(())
    }
}

impl<D: Database + ?Sized> SimpleProfileDb for D {}

#[derive(Debug, thiserror::Error)]
pub enum SimpleProfileDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}
