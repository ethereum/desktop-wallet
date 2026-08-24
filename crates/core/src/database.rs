use zeroize::Zeroizing;

/// A byte-oriented key/value store.
///
/// Implementations are treated as untrusted storage: nothing here is assumed to secure data
/// at rest. Encryption is applied by wrapping a `Database` in an encrypting decorator, so a
/// backend never has to implement, or remember to apply, any cryptography of its own.
///
/// Reads return [`Zeroizing`] buffers so decrypted material is wiped when the caller drops
/// it, rather than being left in a freed allocation.
#[async_trait::async_trait]
pub trait Database: Send + Sync {
    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError>;
    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError>;
    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}
