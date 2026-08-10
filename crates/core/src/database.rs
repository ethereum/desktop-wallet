#[async_trait::async_trait]
pub trait Database: Send + Sync {
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DatabaseError>;
    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError>;
    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError>;
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}
