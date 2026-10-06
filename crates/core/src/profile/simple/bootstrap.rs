use std::sync::Arc;

use super::db::{ProfileRecord, SimpleProfileDatabaseError, SimpleProfileDb};
use crate::database::{Database, scoped::ScopedDatabaseExt};

#[derive(Debug, thiserror::Error)]
pub enum ProfileBootstrapError {
    #[error("profile name `{0}` is ambiguous")]
    Ambiguous(String),
    #[error("database error: {0}")]
    Database(#[from] SimpleProfileDatabaseError),
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

// TODO: maybe replace or relocate: the store fns below take the root store and hardcode b"profiles"; a profile index type should own them.
/// Writes a profile pointer and appends it to the instance index. Does not derive keys.
pub async fn bootstrap_profile(
    store: Arc<dyn Database>,
    mnemonic_index: u32,
    profile_index: u32,
    name: Option<String>,
) -> Result<ProfileRecord, ProfileBootstrapError> {
    let index_db = store.clone().scoped(b"profiles");
    let mut profiles = index_db.list_profiles().await?;
    if profiles.iter().any(|profile| {
        profile.mnemonic_index == mnemonic_index && profile.profile_index == profile_index
    }) {
        return Err(ProfileBootstrapError::Duplicate {
            mnemonic_index,
            profile_index,
        });
    }

    let record = ProfileRecord::new(mnemonic_index, profile_index, name);
    record.check_unique_name(&profiles)?;
    let profile_db = store.scoped(profile_scope(mnemonic_index, profile_index).as_bytes());
    profile_db
        .put_pointer(mnemonic_index, profile_index)
        .await?;
    profiles.push(record.clone());
    index_db.put_profiles(&profiles).await?;
    Ok(record)
}

// TODO: maybe replace or relocate: see bootstrap_profile.
pub async fn rename_profile(
    store: Arc<dyn Database>,
    selector: &str,
    new_name: Option<String>,
) -> Result<ProfileRecord, ProfileBootstrapError> {
    let profiles = store.clone().scoped(b"profiles").list_profiles().await?;
    let current = resolve_profile(&profiles, selector)?;
    set_profile_name(
        store,
        current.mnemonic_index,
        current.profile_index,
        new_name,
    )
    .await
}

// TODO: maybe replace or relocate: see bootstrap_profile.
pub async fn set_profile_name(
    store: Arc<dyn Database>,
    mnemonic_index: u32,
    profile_index: u32,
    name: Option<String>,
) -> Result<ProfileRecord, ProfileBootstrapError> {
    let index_db = store.scoped(b"profiles");
    let mut profiles = index_db.list_profiles().await?;
    let updated = {
        let Some(record) = profiles.iter_mut().find(|profile| {
            profile.mnemonic_index == mnemonic_index && profile.profile_index == profile_index
        }) else {
            return Err(ProfileBootstrapError::Unresolved(format!(
                "{mnemonic_index}/{profile_index}"
            )));
        };
        *record = ProfileRecord::new(mnemonic_index, profile_index, name);
        record.clone()
    };
    updated.check_unique_name(&profiles)?;
    index_db.put_profiles(&profiles).await?;
    Ok(updated)
}

// TODO: maybe replace or relocate: lookup on a slice of ProfileRecord; a method on a collection type.
pub fn resolve_profile<'a>(
    profiles: &'a [ProfileRecord],
    selector: &str,
) -> Result<&'a ProfileRecord, ProfileBootstrapError> {
    if let Some((mnemonic_index, profile_index)) = parse_profile_pair(selector)
        && let Some(record) = profiles.iter().find(|profile| {
            profile.mnemonic_index == mnemonic_index && profile.profile_index == profile_index
        })
    {
        return Ok(record);
    }

    let named: Vec<_> = profiles
        .iter()
        .filter(|profile| profile.name.as_deref() == Some(selector))
        .collect();
    match named.as_slice() {
        [record] => return Ok(record),
        [] => {}
        _ => return Err(ProfileBootstrapError::Ambiguous(selector.to_string())),
    }

    let displayed: Vec<_> = profiles
        .iter()
        .filter(|profile| profile.display_name() == selector)
        .collect();
    match displayed.as_slice() {
        [record] => Ok(record),
        [] => Err(ProfileBootstrapError::Unresolved(selector.to_string())),
        _ => Err(ProfileBootstrapError::Ambiguous(selector.to_string())),
    }
}

// TODO: maybe replace or relocate: lookup on a slice of ProfileRecord; a method on a collection type.
#[must_use]
pub fn next_profile_index(profiles: &[ProfileRecord], mnemonic_index: u32) -> u32 {
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
}

// TODO: maybe replace or relocate: scope key for a profile; a ProfileRecord method or a key type.
#[must_use]
pub fn profile_scope(mnemonic_index: u32, profile_index: u32) -> String {
    format!("profile:{mnemonic_index}:{profile_index}")
}

// TODO: maybe replace or relocate: FromStr on a profile selector type.
fn parse_profile_pair(selector: &str) -> Option<(u32, u32)> {
    let (left, right) = selector.split_once('/')?;
    Some((left.parse().ok()?, right.parse().ok()?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::database::{Database, memory::MemoryDatabase};

    #[tokio::test]
    async fn bootstrap_rejects_duplicate_pair_and_display_name() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        bootstrap_profile(store.clone(), 0, 0, None).await.unwrap();

        let duplicate_pair = bootstrap_profile(store.clone(), 0, 0, Some("work".into()))
            .await
            .unwrap_err();
        assert!(matches!(
            duplicate_pair,
            ProfileBootstrapError::Duplicate {
                mnemonic_index: 0,
                profile_index: 0
            }
        ));

        let duplicate_name = bootstrap_profile(store, 1, 0, None).await.unwrap_err();
        assert!(matches!(
            duplicate_name,
            ProfileBootstrapError::DuplicateName(name) if name == "default"
        ));
    }

    #[tokio::test]
    async fn rename_rejects_taken_display_name() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        bootstrap_profile(store.clone(), 0, 0, Some("work".into()))
            .await
            .unwrap();
        bootstrap_profile(store.clone(), 0, 1, Some("travel".into()))
            .await
            .unwrap();

        let error = set_profile_name(store, 0, 1, Some("work".into()))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProfileBootstrapError::DuplicateName(name) if name == "work"
        ));
    }

    #[test]
    fn next_profile_index_fills_holes() {
        let profiles = vec![
            ProfileRecord {
                mnemonic_index: 0,
                profile_index: 0,
                name: None,
            },
            ProfileRecord {
                mnemonic_index: 0,
                profile_index: 3,
                name: Some("cold".into()),
            },
        ];
        assert_eq!(next_profile_index(&profiles, 0), 1);
        assert_eq!(next_profile_index(&profiles, 1), 0);
    }
}
