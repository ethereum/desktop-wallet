use ethereum_desktop_wallet_core::{
    database::{Database, DatabaseError},
    executor::ExecutorId,
    vault::VaultId,
};

pub trait SimpleProfileDb: Database {
    async fn get_executors(&self) -> Result<Vec<(String, ExecutorId)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"executors").await? else {
            return Ok(vec![]);
        };
        let executors: Vec<(String, ExecutorId)> = postcard::from_bytes(&bytes)?;
        Ok(executors)
    }

    async fn get_vaults(&self) -> Result<Vec<(String, VaultId)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"vaults").await? else {
            return Ok(vec![]);
        };
        let vaults: Vec<(String, VaultId)> = postcard::from_bytes(&bytes)?;
        Ok(vaults)
    }

    async fn put_executors(
        &self,
        executors: &[(&'static str, ExecutorId)],
    ) -> Result<(), SimpleProfileDatabaseError> {
        let bytes = postcard::to_stdvec(executors)?;
        self.put(b"executors", &bytes).await?;
        Ok(())
    }

    async fn put_vaults(
        &self,
        vaults: &[(&'static str, VaultId)],
    ) -> Result<(), SimpleProfileDatabaseError> {
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
