use std::sync::Arc;

use super::db::{ProfileDatabaseError, ProfileDb, ProfileRecord};
use crate::{
    account::{AccountKind, AccountRecord},
    database::{Database, scoped::ScopedDatabase},
};

/// The instance's profiles: the index of records, plus a scope of its own per profile.
pub struct ProfileIndex {
    store: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile name `{0}` is ambiguous")]
    Ambiguous(String),
    #[error("database error: {0}")]
    Database(#[from] ProfileDatabaseError),
    #[error("profile {mnemonic_index}/{profile_index} already exists")]
    Duplicate {
        mnemonic_index: u32,
        profile_index: u32,
    },
    #[error("profile name `{0}` is already used")]
    DuplicateName(String),
    #[error("no profile matches `{0}`")]
    Unresolved(String),
}

impl ProfileIndex {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }

    pub async fn list(&self) -> Result<Vec<ProfileRecord>, ProfileError> {
        Ok(self.records().list_profiles().await?)
    }

    /// The profile `selector` names: `mnemonic/profile` first, then a unique name, then a
    /// unique display name.
    pub async fn find(&self, selector: &str) -> Result<ProfileRecord, ProfileError> {
        let profiles = self.list().await?;
        let pair = selector
            .split_once('/')
            .and_then(|(left, right)| Some((left.parse().ok()?, right.parse().ok()?)));
        if let Some(record) =
            pair.and_then(|pair| profiles.iter().find(|profile| profile.key() == pair))
        {
            return Ok(record.clone());
        }

        let named: Vec<_> = profiles
            .iter()
            .filter(|profile| profile.name.as_deref() == Some(selector))
            .collect();
        match named.as_slice() {
            [record] => return Ok((*record).clone()),
            [] => {}
            _ => return Err(ProfileError::Ambiguous(selector.to_string())),
        }

        let displayed: Vec<_> = profiles
            .iter()
            .filter(|profile| profile.display_name() == selector)
            .collect();
        match displayed.as_slice() {
            [record] => Ok((*record).clone()),
            [] => Err(ProfileError::Unresolved(selector.to_string())),
            _ => Err(ProfileError::Ambiguous(selector.to_string())),
        }
    }

    /// Creates a profile on mnemonic `mnemonic_index`, at `profile_index` or else at the
    /// smallest unused one. Creates no accounts.
    pub async fn create(
        &self,
        mnemonic_index: u32,
        profile_index: Option<u32>,
        name: Option<String>,
    ) -> Result<ProfileRecord, ProfileError> {
        let mut profiles = self.list().await?;
        let profile_index = profile_index.unwrap_or_else(|| {
            let mut used: Vec<u32> = profiles
                .iter()
                .filter(|profile| profile.mnemonic_index == mnemonic_index)
                .map(|profile| profile.profile_index)
                .collect();
            used.sort_unstable();
            let mut next = 0;
            for index in used {
                if index == next {
                    next = next.saturating_add(1);
                } else if index > next {
                    break;
                }
            }
            next
        });
        if profiles
            .iter()
            .any(|profile| profile.key() == (mnemonic_index, profile_index))
        {
            return Err(ProfileError::Duplicate {
                mnemonic_index,
                profile_index,
            });
        }

        let record = ProfileRecord::new(mnemonic_index, profile_index, name);
        record.check_unique_name(&profiles)?;
        profiles.push(record.clone());
        self.records().put_profiles(&profiles).await?;
        Ok(record)
    }

    /// Fails like [`Self::create`] would for `candidate`, without writing anything.
    pub async fn check_name_free(&self, candidate: &ProfileRecord) -> Result<(), ProfileError> {
        candidate.check_unique_name(&self.list().await?)
    }

    pub async fn accounts(
        &self,
        profile: &ProfileRecord,
    ) -> Result<Vec<AccountRecord>, ProfileError> {
        Ok(self.profile_scope(profile).get_accounts().await?)
    }

    /// Adds the accounts in `kinds` the profile does not have yet, and returns those.
    pub async fn add_accounts(
        &self,
        profile: &ProfileRecord,
        kinds: Vec<AccountKind>,
    ) -> Result<Vec<AccountRecord>, ProfileError> {
        let scope = self.profile_scope(profile);
        let mut accounts = scope.get_accounts().await?;
        let mut added = Vec::new();
        for kind in kinds {
            if accounts
                .iter()
                .any(|account| account.kind.same_branch(&kind))
            {
                continue;
            }
            let id = accounts
                .iter()
                .map(|account| account.id.saturating_add(1))
                .max()
                .unwrap_or(0);
            let account = AccountRecord {
                id,
                kind,
                label: None,
            };
            accounts.push(account.clone());
            added.push(account);
        }
        if !added.is_empty() {
            scope.put_accounts(&accounts).await?;
        }
        Ok(added)
    }

    pub async fn rename(
        &self,
        selector: &str,
        name: Option<String>,
    ) -> Result<ProfileRecord, ProfileError> {
        let (mnemonic_index, profile_index) = self.find(selector).await?.key();
        self.set_name(mnemonic_index, profile_index, name).await
    }

    pub async fn set_name(
        &self,
        mnemonic_index: u32,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<ProfileRecord, ProfileError> {
        let mut profiles = self.list().await?;
        let Some(record) = profiles
            .iter_mut()
            .find(|profile| profile.key() == (mnemonic_index, profile_index))
        else {
            return Err(ProfileError::Unresolved(format!(
                "{mnemonic_index}/{profile_index}"
            )));
        };
        *record = ProfileRecord::new(mnemonic_index, profile_index, name);
        let updated = record.clone();
        updated.check_unique_name(&profiles)?;
        self.records().put_profiles(&profiles).await?;
        Ok(updated)
    }

    fn records(&self) -> ScopedDatabase {
        ScopedDatabase::new(self.store.clone(), b"profiles")
    }

    fn profile_scope(&self, profile: &ProfileRecord) -> ScopedDatabase {
        ScopedDatabase::new(self.store.clone(), profile.scope().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory::MemoryDatabase;

    fn index() -> ProfileIndex {
        ProfileIndex::new(Arc::new(MemoryDatabase::new()))
    }

    #[tokio::test]
    async fn create_rejects_a_duplicate_pair_and_a_taken_display_name() {
        let profiles = index();
        profiles.create(0, Some(0), None).await.unwrap();

        let duplicate_pair = profiles
            .create(0, Some(0), Some("work".into()))
            .await
            .unwrap_err();
        assert!(matches!(
            duplicate_pair,
            ProfileError::Duplicate {
                mnemonic_index: 0,
                profile_index: 0
            }
        ));

        let duplicate_name = profiles.create(1, Some(0), None).await.unwrap_err();
        assert!(matches!(
            duplicate_name,
            ProfileError::DuplicateName(name) if name == "default"
        ));
    }

    #[tokio::test]
    async fn set_name_rejects_a_taken_display_name() {
        let profiles = index();
        profiles
            .create(0, Some(0), Some("work".into()))
            .await
            .unwrap();
        profiles
            .create(0, Some(1), Some("travel".into()))
            .await
            .unwrap();

        let error = profiles
            .set_name(0, 1, Some("work".into()))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProfileError::DuplicateName(name) if name == "work"
        ));
    }

    #[tokio::test]
    async fn create_without_an_index_fills_the_first_hole_on_that_mnemonic() {
        let profiles = index();
        profiles.create(0, Some(0), None).await.unwrap();
        profiles
            .create(0, Some(3), Some("cold".into()))
            .await
            .unwrap();

        assert_eq!(
            profiles
                .create(0, None, Some("a".into()))
                .await
                .unwrap()
                .profile_index,
            1
        );
        assert_eq!(
            profiles
                .create(1, None, Some("b".into()))
                .await
                .unwrap()
                .profile_index,
            0
        );
    }

    #[tokio::test]
    async fn find_prefers_the_pair_then_the_name_then_the_display_name() {
        let profiles = index();
        profiles
            .create(0, Some(0), Some("work".into()))
            .await
            .unwrap();
        profiles.create(0, Some(1), None).await.unwrap();

        assert_eq!(profiles.find("0/1").await.unwrap().key(), (0, 1));
        assert_eq!(profiles.find("work").await.unwrap().key(), (0, 0));
        assert_eq!(profiles.find("profile #1").await.unwrap().key(), (0, 1));
        assert!(matches!(
            profiles.find("nobody").await,
            Err(ProfileError::Unresolved(_))
        ));
    }
}
