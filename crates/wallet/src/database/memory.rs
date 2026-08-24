use std::sync::Mutex;

use edw_core::database::{Database, DatabaseError};
use zeroize::Zeroizing;

#[derive(Default)]
pub struct MemoryDatabase {
    store: Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>,
}

#[derive(Debug, thiserror::Error)]
#[error("Memory database error")]
pub struct MemoryDatabaseError;

impl MemoryDatabase {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every key currently held, in unspecified order.
    ///
    /// [`Database`] has no iteration, so this is the seam tests use to inspect what a
    /// backend actually stored.
    ///
    /// # Errors
    /// Returns an error if the store's lock is poisoned.
    pub fn keys(&self) -> Result<Vec<Vec<u8>>, DatabaseError> {
        let store = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Other(Box::new(MemoryDatabaseError)))?;
        Ok(store.keys().cloned().collect())
    }
}

#[async_trait::async_trait]
impl Database for MemoryDatabase {
    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError> {
        let store = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Other(Box::new(MemoryDatabaseError)))?;
        Ok(store.get(key).cloned().map(Zeroizing::new))
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Other(Box::new(MemoryDatabaseError)))?;
        store.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Other(Box::new(MemoryDatabaseError)))?;
        store.remove(key);
        Ok(())
    }
}
