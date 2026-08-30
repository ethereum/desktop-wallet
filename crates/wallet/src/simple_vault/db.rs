use alloy_primitives::Address;
use edw_core::database::{Database, DatabaseError};

pub trait SimpleVaultDb: Database {
    async fn get_implementation(&self) -> Result<Address, SimpleVaultDatabaseError> {
        let Some(bytes) = self.get(b"implementation").await? else {
            return Err(SimpleVaultDatabaseError::MissingImplementation);
        };
        Address::try_from(bytes.as_slice())
            .map_err(|_| SimpleVaultDatabaseError::InvalidImplementation)
    }

    async fn put_implementation(
        &self,
        implementation: &Address,
    ) -> Result<(), SimpleVaultDatabaseError> {
        self.put(b"implementation", implementation.as_slice())
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleVaultDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("missing implementation address")]
    MissingImplementation,
    #[error("invalid implementation address")]
    InvalidImplementation,
}

impl<D: Database + ?Sized> SimpleVaultDb for D {}
