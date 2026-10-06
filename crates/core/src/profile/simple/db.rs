use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::bootstrap::ProfileBootstrapError;
use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait SimpleProfileDb: Database {
    async fn get_executor(&self) -> Result<Option<(Uuid, String)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"executor").await? else {
            return Ok(None);
        };
        let executor = postcard::from_bytes(&bytes)?;
        Ok(Some(executor))
    }

    async fn get_pointer(&self) -> Result<Option<(u32, u32)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"pointer").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn get_vaults(&self) -> Result<Vec<(Uuid, String)>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"vaults").await? else {
            return Ok(vec![]);
        };
        let vaults = postcard::from_bytes(&bytes)?;
        Ok(vaults)
    }

    async fn put_executor(&self, executor: (Uuid, &str)) -> Result<(), SimpleProfileDatabaseError> {
        let bytes = postcard::to_stdvec(&executor)?;
        self.put(b"executor", &bytes).await?;
        Ok(())
    }

    async fn put_pointer(
        &self,
        mnemonic_index: u32,
        profile_index: u32,
    ) -> Result<(), SimpleProfileDatabaseError> {
        self.put(
            b"pointer",
            &postcard::to_stdvec(&(mnemonic_index, profile_index))?,
        )
        .await?;
        Ok(())
    }

    async fn put_vaults(&self, vaults: &[(Uuid, &str)]) -> Result<(), SimpleProfileDatabaseError> {
        let bytes = postcard::to_stdvec(vaults)?;
        self.put(b"vaults", &bytes).await?;
        Ok(())
    }

    async fn list_profiles(&self) -> Result<Vec<ProfileRecord>, SimpleProfileDatabaseError> {
        let Some(bytes) = self.get(b"index").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_profiles(
        &self,
        profiles: &[ProfileRecord],
    ) -> Result<(), SimpleProfileDatabaseError> {
        self.put(b"index", &postcard::to_stdvec(profiles)?).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRecord {
    pub mnemonic_index: u32,
    pub profile_index: u32,
    pub name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleProfileDatabaseError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
}

impl<D: Database + ?Sized> SimpleProfileDb for D {}

impl ProfileRecord {
    /// Treats an empty name or `-` as no name.
    #[must_use]
    pub fn new(mnemonic_index: u32, profile_index: u32, name: Option<String>) -> Self {
        Self {
            mnemonic_index,
            profile_index,
            name: name.filter(|name| !name.is_empty() && name != "-"),
        }
    }

    /// Rejects `self` if another profile in `profiles` has the same display name.
    pub fn check_unique_name(&self, profiles: &[Self]) -> Result<(), ProfileBootstrapError> {
        let display = self.display_name();
        let taken = profiles.iter().any(|profile| {
            (profile.mnemonic_index, profile.profile_index)
                != (self.mnemonic_index, self.profile_index)
                && profile.display_name() == display
        });
        if taken {
            Err(ProfileBootstrapError::DuplicateName(display))
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        match self.name.as_deref() {
            Some(name) if !name.is_empty() => name.to_string(),
            _ if self.profile_index == 0 => "default".to_string(),
            _ => format!("profile #{}", self.profile_index),
        }
    }
}
