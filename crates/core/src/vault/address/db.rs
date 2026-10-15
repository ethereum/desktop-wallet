use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait AddressVaultDb: Database {
    async fn get_derived_address(&self) -> Result<DerivedAddress, AddressVaultDatabaseError> {
        let Some(bytes) = self.get(b"address").await? else {
            return Err(AddressVaultDatabaseError::MissingAddress);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_derived_address(
        &self,
        derived: &DerivedAddress,
    ) -> Result<(), AddressVaultDatabaseError> {
        self.put(b"address", &postcard::to_stdvec(derived)?).await?;
        Ok(())
    }
}

/// A profile's standard address and where it was derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedAddress {
    pub address: Address,
    /// `i` in `m/44'/60'/x'/0/i`.
    pub index: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum AddressVaultDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("missing address")]
    MissingAddress,
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

impl<D: Database + ?Sized> AddressVaultDb for D {}
