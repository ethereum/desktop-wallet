use std::{
    env, fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use edw_core::network::SupportedNetwork;
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_mins(15);

pub(crate) struct Session {
    pub(crate) data_dir: PathBuf,
    pub(crate) network: SupportedNetwork,
    pub(crate) password: Zeroizing<String>,
}

pub(crate) fn canonical_data_dir(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut suffix = PathBuf::new();
    let mut cursor = absolute.as_path();
    loop {
        if let Ok(canonical) = fs::canonicalize(cursor) {
            return if suffix.as_os_str().is_empty() {
                canonical
            } else {
                canonical.join(suffix)
            };
        }
        match (cursor.file_name(), cursor.parent()) {
            (Some(name), Some(parent)) if parent != cursor => {
                suffix = Path::new(name).join(suffix);
                cursor = parent;
            }
            _ => return absolute,
        }
    }
}

fn directory() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|slot| slot.borrow().clone())
        && fs::create_dir_all(&dir).is_ok()
        && fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).is_ok()
    {
        return Some(dir);
    }

    let tmp = env::var_os("TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    [env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from), Some(tmp)]
        .into_iter()
        .flatten()
        .map(|root| root.join("edw"))
        .find(|dir| {
            fs::create_dir_all(dir).is_ok()
                && fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).is_ok()
        })
}

fn path() -> Option<PathBuf> {
    Some(directory()?.join("session"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn load() -> Option<Session> {
    let path = path()?;
    let raw = Zeroizing::new(fs::read_to_string(&path).ok()?);
    let mut parts = raw.splitn(4, '\n');
    let deadline = parts.next()?;
    let data_dir = parts.next()?;
    let network = parts.next()?;
    let password = parts.next()?;

    if now() >= deadline.parse::<u64>().ok()? {
        let _ = fs::remove_file(&path);
        return None;
    }

    let session = Session {
        data_dir: PathBuf::from(data_dir),
        network: network.parse().ok()?,
        password: Zeroizing::new(password.to_string()),
    };
    let _ = store(&session);
    Some(session)
}

pub(crate) fn store(session: &Session) -> Result<(), anyhow::Error> {
    let directory = directory().context("no writable runtime directory for a session")?;
    let path = directory.join("session");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("error writing {}", path.display()))?;

    write!(
        file,
        "{}\n{}\n{}\n{}",
        now() + TTL.as_secs(),
        session.data_dir.display(),
        session.network,
        session.password.as_str()
    )?;
    Ok(())
}

pub(crate) fn clear() -> Result<bool, anyhow::Error> {
    let Some(path) = path() else {
        return Ok(false);
    };
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("error removing {}", path.display())),
    }
}

#[cfg(test)]
thread_local! {
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn isolated(test: impl FnOnce()) {
        let dir = std::env::temp_dir().join(format!(
            "edw-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        TEST_DIR.with(|slot| *slot.borrow_mut() = Some(dir.clone()));
        test();
        TEST_DIR.with(|slot| *slot.borrow_mut() = None);
        let _ = fs::remove_dir_all(dir);
    }

    fn sample(network: SupportedNetwork) -> Session {
        Session {
            data_dir: PathBuf::from("/tmp/edw-data"),
            network,
            password: Zeroizing::new("secret".into()),
        }
    }

    #[test]
    fn canonical_data_dir_resolves_a_missing_leaf() {
        let parent = std::env::temp_dir();
        let missing = parent.join("edw-missing-leaf");
        assert_eq!(
            canonical_data_dir(&missing),
            fs::canonicalize(&parent).unwrap().join("edw-missing-leaf")
        );
    }

    #[test]
    fn load_returns_what_was_stored() {
        isolated(|| {
            store(&sample(SupportedNetwork::Sepolia)).unwrap();
            let loaded = load().unwrap();
            assert_eq!(loaded.network, SupportedNetwork::Sepolia);
            assert_eq!(loaded.password.as_str(), "secret");
        });
    }

    #[test]
    fn store_replaces_the_unlocked_network() {
        isolated(|| {
            store(&sample(SupportedNetwork::Sepolia)).unwrap();
            store(&sample(SupportedNetwork::Mainnet)).unwrap();
            assert_eq!(load().unwrap().network, SupportedNetwork::Mainnet);
        });
    }
}
