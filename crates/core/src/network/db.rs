use super::NetworkConfig;
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait NetworkDb: Database {
    async fn get_network_configs(&self) -> Result<Vec<NetworkConfig>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"networkConfigs").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_network_configs(
        &self,
        configs: &[NetworkConfig],
    ) -> Result<(), NetworkDatabaseError> {
        self.put(b"networkConfigs", &postcard::to_stdvec(configs)?)
            .await?;
        Ok(())
    }

    async fn get_active(&self) -> Result<Option<String>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"active").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn put_active(&self, name: &str) -> Result<(), NetworkDatabaseError> {
        self.put(b"active", &postcard::to_stdvec(&name)?).await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> NetworkDb for D {}
