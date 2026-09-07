use super::{Network, NetworkId};
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait NetworkDb: Database {
    async fn get_networks(&self) -> Result<Vec<Network>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"networks").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_networks(&self, networks: &[Network]) -> Result<(), NetworkDatabaseError> {
        self.put(b"networks", &postcard::to_stdvec(networks)?)
            .await?;
        Ok(())
    }

    async fn get_active(&self) -> Result<Option<NetworkId>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"active").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn put_active(&self, id: NetworkId) -> Result<(), NetworkDatabaseError> {
        self.put(b"active", &postcard::to_stdvec(&id)?).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> NetworkDb for D {}

#[derive(Debug, thiserror::Error)]
pub enum NetworkDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}
