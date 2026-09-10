use zeroize::Zeroizing;

pub mod encrypted;
pub mod file;
pub mod memory;
pub mod scoped;

/// A byte-oriented key/value store.
///
/// Backends are untrusted: encryption is a decorator, not a backend responsibility. Reads
/// return [`Zeroizing`] buffers so decrypted material is wiped when dropped.
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
