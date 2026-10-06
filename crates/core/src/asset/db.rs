use alloy_primitives::Address;

use super::Erc20Asset;
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait AssetDb: Database {
    /// ERC-20s registered wallet-wide beyond the chain defaults. Kept in the preferences scope.
    async fn get_erc20s(&self) -> Result<Vec<Erc20Asset>, AssetDatabaseError> {
        let Some(bytes) = self.get(b"erc20s").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    /// ERC-20 addresses a profile opted into. Kept in the profile's scope.
    async fn get_opted_in(&self) -> Result<Vec<Address>, AssetDatabaseError> {
        let Some(bytes) = self.get(b"optedIn").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_erc20s(&self, assets: &[Erc20Asset]) -> Result<(), AssetDatabaseError> {
        self.put(b"erc20s", &postcard::to_stdvec(assets)?).await?;
        Ok(())
    }

    async fn put_opted_in(&self, addresses: &[Address]) -> Result<(), AssetDatabaseError> {
        self.put(b"optedIn", &postcard::to_stdvec(addresses)?)
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssetDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> AssetDb for D {}
