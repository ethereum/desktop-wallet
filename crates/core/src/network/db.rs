use super::{Network, NetworkEndpointConfig};
use crate::{
    asset::Asset,
    database::{Database, DatabaseError},
};

#[async_trait::async_trait]
pub trait NetworkDb: Database {
    async fn get_network(&self) -> Result<Option<Network>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"network").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn put_network(&self, network: &Network) -> Result<(), NetworkDatabaseError> {
        self.put(b"network", &postcard::to_stdvec(network)?).await?;
        Ok(())
    }

    async fn get_endpoint_configs(
        &self,
    ) -> Result<Vec<NetworkEndpointConfig>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"endpointConfigs").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_endpoint_configs(
        &self,
        configs: &[NetworkEndpointConfig],
    ) -> Result<(), NetworkDatabaseError> {
        self.put(b"endpointConfigs", &postcard::to_stdvec(configs)?)
            .await?;
        Ok(())
    }

    async fn get_active_endpoint(&self) -> Result<Option<String>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"activeEndpoint").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn put_active_endpoint(&self, name: &str) -> Result<(), NetworkDatabaseError> {
        self.put(b"activeEndpoint", &postcard::to_stdvec(&name)?)
            .await?;
        Ok(())
    }

    async fn get_assets(&self) -> Result<Vec<Asset>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"assets").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_assets(&self, assets: &[Asset]) -> Result<(), NetworkDatabaseError> {
        self.put(b"assets", &postcard::to_stdvec(assets)?).await?;
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
