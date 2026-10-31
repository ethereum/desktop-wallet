use zeroize::Zeroizing;

pub mod encrypted;
pub mod file;
pub mod memory;
pub mod scoped;

static MEMORY_OPERATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Held across a cooperating read/modify/write operation; released on drop.
pub type DatabaseGuard = Box<dyn Send + Sync>;

/// A byte-oriented key/value store.
///
/// Backends are untrusted: encryption is a decorator, not a backend responsibility. Reads
/// return [`Zeroizing`] buffers so decrypted material is wiped when dropped.
#[async_trait::async_trait]
pub trait Database: Send + Sync {
    /// Serializes compound operations. Decorators must forward this to their backend.
    /// The default coordinates in-process backends; persistent backends must also lock
    /// across processes. Ordinary get/put calls do not acquire this lock themselves.
    async fn lock(&self) -> Result<DatabaseGuard, DatabaseError> {
        Ok(Box::new(MEMORY_OPERATION_LOCK.lock().await))
    }

    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError>;
    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError>;
    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}
