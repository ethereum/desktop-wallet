use std::sync::Mutex;

use ethereum_desktop_wallet_core::database::{Database, DatabaseError};

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
}

#[async_trait::async_trait]
impl Database for MemoryDatabase {
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DatabaseError> {
        let store = self
            .store
            .lock()
            .map_err(|_| DatabaseError::Other(Box::new(MemoryDatabaseError)))?;
        Ok(store.get(key).cloned())
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
