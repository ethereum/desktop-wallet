use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::database::{Database, DatabaseError};

/// The list of a profile's vaults. Kept in the profile's scope.
#[async_trait::async_trait]
pub trait VaultDb: Database {
    async fn vault_records(&self) -> Result<Vec<VaultRecord>, VaultDatabaseError> {
        let Some(bytes) = self.get(b"vaults").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_vault_records(&self, records: &[VaultRecord]) -> Result<(), VaultDatabaseError> {
        self.put(b"vaults", &postcard::to_stdvec(records)?).await?;
        Ok(())
    }
}

/// Which factory builds a vault, and the scope its own records live in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultRecord {
    pub tag: String,
    pub id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum VaultDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> VaultDb for D {}

impl VaultRecord {
    /// A record for a new vault of `tag`, with a scope no other vault uses.
    #[must_use]
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            id: Uuid::new_v4(),
        }
    }

    /// The storage scope of this vault's own records, within its profile's scope.
    #[must_use]
    pub fn scope(&self) -> String {
        format!("vault:{}", self.id)
    }
}
