use alloy_primitives::Address;
use edw_core::database::{Database, DatabaseError};

pub trait SimpleExecutorDb: Database {
    async fn get_implementation(&self) -> Result<Address, SimpleExecutorDatabaseError> {
        let Some(bytes) = self.get(b"implementation").await? else {
            return Err(SimpleExecutorDatabaseError::MissingImplementation);
        };
        Address::try_from(bytes.as_slice())
            .map_err(|_| SimpleExecutorDatabaseError::InvalidImplementation)
    }

    async fn put_implementation(
        &self,
        implementation: &Address,
    ) -> Result<(), SimpleExecutorDatabaseError> {
        self.put(b"implementation", implementation.as_slice())
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleExecutorDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("missing implementation address")]
    MissingImplementation,
    #[error("invalid implementation address")]
    InvalidImplementation,
}

impl<D: Database + ?Sized> SimpleExecutorDb for D {}
