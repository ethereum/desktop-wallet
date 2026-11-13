use std::{future::Future, sync::Arc};

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
        self.fresh_address_with(profile, endpoint, |index| async move {
            Ok(mnemonic.standard_address(index, profile.profile_index)?)
        })
        .await
    }

    /// [`Self::fresh_address`] over the addresses `address_at` returns.
    pub async fn fresh_address_with<E, A, AFut>(
        &self,
        profile: &ProfileRecord,
        endpoint: &dyn NetworkEndpoint,
        mut address_at: A,
    ) -> Result<FreshAddress, E>
    where
        E: From<InstanceError>,
        A: FnMut(u32) -> AFut,
        AFut: Future<Output = Result<Address, E>>,
    {
        let mut index = self
            .profile_db(profile)
            .next_address()
            .await
            .map_err(|error| InstanceError::from(ProfileError::from(error)))?;
        loop {
            if index >= MAX_INDEX {
                return Err(
                    InstanceError::Mnemonic(crate::mnemonic::MnemonicError::ScanLimit(MAX_INDEX))
                        .into(),
                );
            }
            let address = address_at(index).await?;
            if !endpoint
                .has_activity(address)
                .await
                .map_err(InstanceError::from)?
            {
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

#[cfg(test)]
mod tests {
    use alloy_primitives::{Bytes, U64, U256};
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{network::NetworkId, test_support::mocked_provider};

    #[tokio::test]
    async fn the_fresh_address_is_the_first_source_address_without_activity() {
        let instance = Instance::in_memory(NetworkId(1337));
        let mock = Asserter::new();
        mock.push_success(&U64::from(1)); // Address 0 has sent a transaction.
        mock.push_success(&U64::ZERO);
        mock.push_success(&Bytes::new());
        mock.push_success(&U256::ZERO);

        let fresh = instance
            .fresh_address_with(
                &ProfileRecord::new(0, 0, None),
                mocked_provider(&mock).as_ref(),
                |index| async move { Ok::<_, InstanceError>(Address::with_last_byte(index as u8)) },
            )
            .await
            .unwrap();

        assert_eq!(fresh.index, 1);
        assert_eq!(fresh.address, Address::with_last_byte(1));
    }
}
