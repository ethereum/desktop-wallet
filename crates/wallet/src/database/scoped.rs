use std::sync::Arc;

use edw_core::database::{Database, DatabaseError};
use zeroize::Zeroizing;

/// Confines an inner [`Database`] to a keyspace, so each vault and executor in a profile
/// reads and writes under its own scope.
///
/// Scoping alone is namespacing, not isolation. Layer this over an
/// [`super::encrypted::EncryptedDatabase`] to get the cryptographic half: the scope becomes
/// part of the logical key, which feeds per-record key derivation and the AEAD's associated
/// data, so records cannot be moved between scopes.
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

    /// Length-prefixes the scope so no two (scope, key) pairs can produce the same composed
    /// key. Plain concatenation would let scope `a` key `bc` collide with scope `ab` key `c`.
    fn scoped_key(&self, key: &[u8]) -> Vec<u8> {
        let len = self.prefix.len() as u32;
        let mut composed = Vec::with_capacity(size_of::<u32>() + self.prefix.len() + key.len());
        composed.extend_from_slice(&len.to_le_bytes());
        composed.extend_from_slice(&self.prefix);
        composed.extend_from_slice(key);
        composed
    }
}

#[async_trait::async_trait]
impl Database for ScopedDatabase {
    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError> {
        self.db.get(&self.scoped_key(key)).await
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        self.db.put(&self.scoped_key(key), value).await
    }

    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError> {
        self.db.delete(&self.scoped_key(key)).await
    }
}
