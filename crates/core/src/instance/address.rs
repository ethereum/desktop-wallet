use std::sync::Arc;

use alloy_primitives::Address;

use super::{Instance, InstanceError};
use crate::{
    database::{Database, scoped::ScopedDatabase},
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    profile::{ProfileDb, ProfileError, ProfileRecord},
};

const MAX_INDEX: u32 = 1000;

/// A standard address this profile has not handed out, and that has no on-chain activity.
pub struct FreshAddress {
    pub index: u32,
    pub address: Address,
}

impl Instance {
    /// The next clean unstored standard address for `profile`.
    ///
    /// Does not store it. A failed command can ask again and receive the same address.
    ///
    /// # Errors
    /// Returns an error if every index up to [`MAX_INDEX`] has been stored or has activity.
    pub async fn fresh_address(
        &self,
        profile: &ProfileRecord,
        mnemonic: &Mnemonic,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<FreshAddress, InstanceError> {
        let mut index = self
            .profile_db(profile)
            .next_address()
            .await
            .map_err(ProfileError::from)?;
        loop {
            if index >= MAX_INDEX {
                return Err(InstanceError::Mnemonic(
                    crate::mnemonic::MnemonicError::ScanLimit(MAX_INDEX),
                ));
            }
            let address = mnemonic.standard_address(index, profile.profile_index)?;
            if !endpoint.has_activity(address).await? {
                return Ok(FreshAddress { index, address });
            }
            index = index.saturating_add(1);
        }
    }

    /// Records `index` as handed out, and every lower unstored index with it.
    ///
    /// # Errors
    /// Returns an error if the profile record cannot be written.
    pub async fn commit_fresh_address(
        &self,
        profile: &ProfileRecord,
        index: u32,
    ) -> Result<(), InstanceError> {
        let db = self.profile_db(profile);
        let current = db.next_address().await.map_err(ProfileError::from)?;
        let next = index.saturating_add(1);
        if next > current {
            db.put_next_address(next)
                .await
                .map_err(ProfileError::from)?;
        }
        Ok(())
    }

    /// The profile scope, then a `tornado` subscope for notes and the sync cursor.
    #[must_use]
    pub fn tornado_database(&self, profile: &ProfileRecord) -> Arc<dyn Database> {
        let profile_db: Arc<dyn Database> = Arc::new(self.profile_db(profile));
        Arc::new(ScopedDatabase::new(profile_db, b"tornado"))
    }
}
