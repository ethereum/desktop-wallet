use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use edw_core::network::NetworkId;
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_mins(15);

pub struct Session {
    pub data_dir: PathBuf,
    pub network: NetworkId,
    pub password: Zeroizing<String>,
}

impl Session {
    pub fn load() -> Option<Self> {
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

        let session = Self {
            data_dir: PathBuf::from(data_dir),
            network: network.parse().ok()?,
            password: Zeroizing::new(password.to_string()),
        };
        let _ = session.store();
        Some(session)
    }

    // TODO: maybe replace or relocate: duplicates FileDatabase's private dir/file creation.
    pub fn store(&self) -> Result<(), anyhow::Error> {
        let directory =
            directory().context("XDG_RUNTIME_DIR is not set, so no session can be held")?;
        fs::create_dir_all(&directory)
            .with_context(|| format!("error creating {}", directory.display()))?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;

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
            self.data_dir.display(),
            self.network,
            self.password.as_str()
        )?;
        Ok(())
    }

    pub fn clear() -> Result<bool, anyhow::Error> {
        let Some(path) = path() else {
            return Ok(false);
        };
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).with_context(|| format!("error removing {}", path.display())),
        }
    }
}

// TODO: maybe replace or relocate: runtime-dir lookup plus a test hook; Session should own its path.
fn directory() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|slot| slot.borrow().clone()) {
        return Some(dir);
    }

    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("edw"))
}

// TODO: maybe replace or relocate: one-line wrapper over directory().
fn path() -> Option<PathBuf> {
    Some(directory()?.join("session"))
}

// TODO: maybe replace or relocate: store a SystemTime deadline instead of a hand-rolled clock.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

    fn sample(network: NetworkId) -> Session {
        Session {
            data_dir: PathBuf::from("/tmp/edw-data"),
            network,
            password: Zeroizing::new("secret".into()),
        }
    }

    #[test]
    fn load_returns_what_was_stored() {
        isolated(|| {
            sample(NetworkId(11_155_111)).store().unwrap();
            let loaded = Session::load().unwrap();
            assert_eq!(loaded.network, NetworkId(11_155_111));
            assert_eq!(loaded.password.as_str(), "secret");
        });
    }

    #[test]
    fn store_replaces_the_unlocked_network() {
        isolated(|| {
            sample(NetworkId(11_155_111)).store().unwrap();
            sample(NetworkId(1337)).store().unwrap();
            assert_eq!(Session::load().unwrap().network, NetworkId(1337));
        });
    }
}
