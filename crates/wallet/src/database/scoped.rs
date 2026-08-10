use std::sync::Arc;

use ethereum_desktop_wallet_core::database::{Database, DatabaseError};

pub struct ScopedDatabase {
    db: Arc<dyn Database>,
    prefix: Vec<u8>,
}

pub trait ScopedDatabaseExt {
    fn scoped(self, prefix: &[u8]) -> ScopedDatabase;
}

impl ScopedDatabaseExt for Arc<dyn Database> {
    fn scoped(self, prefix: &[u8]) -> ScopedDatabase {
        ScopedDatabase::new(self, prefix)
    }
}

impl ScopedDatabase {
    fn new(db: Arc<dyn Database>, prefix: &[u8]) -> Self {
        Self {
            db,
            prefix: prefix.to_vec(),
        }
    }

    fn scoped_key(&self, key: &[u8]) -> Vec<u8> {
        [self.prefix.as_slice(), key].concat()
    }
}

#[async_trait::async_trait]
impl Database for ScopedDatabase {
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DatabaseError> {
        self.db.get(&self.scoped_key(key)).await
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        self.db.put(&self.scoped_key(key), value).await
    }

    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError> {
        self.db.delete(&self.scoped_key(key)).await
    }
}
