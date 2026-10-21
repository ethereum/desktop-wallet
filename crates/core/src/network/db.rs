use super::{Network, RpcConfig};
use crate::database::{Database, DatabaseError};

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

    async fn get_rpc_configs(&self) -> Result<Vec<RpcConfig>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"rpcConfigs").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_rpc_configs(&self, configs: &[RpcConfig]) -> Result<(), NetworkDatabaseError> {
        self.put(b"rpcConfigs", &postcard::to_stdvec(configs)?)
            .await?;
        Ok(())
    }

    async fn get_active_rpc_config(&self) -> Result<Option<String>, NetworkDatabaseError> {
        let Some(bytes) = self.get(b"activeRpcConfig").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn put_active_rpc_config(&self, name: &str) -> Result<(), NetworkDatabaseError> {
        self.put(b"activeRpcConfig", &postcard::to_stdvec(&name)?)
            .await?;
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
