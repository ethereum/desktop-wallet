use std::{
    ffi::OsString,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use edw_core::{
    database::{Database, file::FileDatabase},
    instance::DataDir,
    network::NetworkId,
};
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_mins(15);

const RECORD: &[u8] = b"session";

const NO_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR is not set, so no session can be held";

/// The unlocked network and its password, kept between commands for [`TTL`] after last use.
pub struct Session {
    pub data_dir: PathBuf,
    pub network: NetworkId,
    pub password: Zeroizing<String>,
}

/// The session store under [`runtime_dir`]: `None` without one, or why it cannot be opened.
pub struct SessionFile(Option<Result<FileDatabase, String>>);

impl SessionFile {
    pub fn runtime() -> Self {
        let Some(dir) = runtime_dir(std::env::var_os("XDG_RUNTIME_DIR")) else {
            return Self(None);
        };
        Self(Some(FileDatabase::open(&dir).map_err(|error| {
            format!(
                "the session store at {} cannot be opened: {error}",
                dir.display()
            )
        })))
    }

    /// Whether this system has a place for a session, even one that cannot be opened.
    pub fn can_hold(&self) -> bool {
        self.0.is_some()
    }

    fn opened(&self) -> Result<Option<&FileDatabase>, anyhow::Error> {
        match &self.0 {
            None => Ok(None),
            Some(Ok(store)) => Ok(Some(store)),
            Some(Err(reason)) => Err(anyhow::anyhow!("{reason}")),
        }
    }

    /// The live session, renewed for another [`TTL`]. An expired one is removed.
    pub async fn load(&self) -> Option<Session> {
        let store = self.opened().ok()??;
        let raw = store.get(RECORD).await.ok()??;
        let mut parts = std::str::from_utf8(&raw).ok()?.splitn(4, '\n');
        let deadline = UNIX_EPOCH.checked_add(Duration::from_secs(parts.next()?.parse().ok()?))?;
        let data_dir = parts.next()?;
        let network = parts.next()?;
        let password = parts.next()?;

        if SystemTime::now() >= deadline {
            let _ = store.delete(RECORD).await;
            return None;
        }

        let session = Session {
            data_dir: PathBuf::from(data_dir),
            network: network.parse().ok()?,
            password: Zeroizing::new(password.to_string()),
        };
        let _ = self.store(&session).await;
        Some(session)
    }

    /// The live session whose wallet instance is still in its data directory.
    ///
    /// A session for an instance that has been removed is deleted, so the wallet is locked.
    pub async fn load_if_present(&self) -> Option<Session> {
        let session = self.load().await?;
        if DataDir::new(&session.data_dir).has_instance(session.network) {
            return Some(session);
        }
        let _ = self.clear().await;
        None
    }

    pub async fn store(&self, session: &Session) -> Result<(), anyhow::Error> {
        let store = self.opened()?.context(NO_RUNTIME_DIR)?;
        let deadline = SystemTime::now()
            .checked_add(TTL)
            .and_then(|deadline| deadline.duration_since(UNIX_EPOCH).ok())
            .context("the system clock is out of range")?
            .as_secs();
        let record = Zeroizing::new(format!(
            "{deadline}\n{}\n{}\n{}",
            session.data_dir.display(),
            session.network,
            session.password.as_str()
        ));
        store.put(RECORD, record.as_bytes()).await?;
        Ok(())
    }

    /// Whether there was a session to remove.
    pub async fn clear(&self) -> Result<bool, anyhow::Error> {
        let Some(store) = self.opened()? else {
            return Ok(false);
        };
        let existed = store.get(RECORD).await?.is_some();
        store.delete(RECORD).await?;
        Ok(existed)
    }
}

/// `$XDG_RUNTIME_DIR/edw`. No `$TMPDIR` fallback: that is on disk, so the password would persist.
fn runtime_dir(xdg_runtime_dir: Option<OsString>) -> Option<PathBuf> {
    xdg_runtime_dir
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("edw"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn isolated() -> (SessionFile, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "edw-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        (
            SessionFile(Some(Ok(FileDatabase::open(&dir).unwrap()))),
            dir,
        )
    }

    fn sample(network: NetworkId) -> Session {
        Session {
            data_dir: PathBuf::from("/tmp/edw-data"),
            network,
            password: Zeroizing::new("secret".into()),
        }
    }

    #[tokio::test]
    async fn load_returns_what_was_stored() {
        let (sessions, dir) = isolated();
        sessions
            .store(&sample(NetworkId(11_155_111)))
            .await
            .unwrap();
        let loaded = sessions.load().await.unwrap();
        assert_eq!(loaded.network, NetworkId(11_155_111));
        assert_eq!(loaded.password.as_str(), "secret");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn store_replaces_the_unlocked_network() {
        let (sessions, dir) = isolated();
        sessions
            .store(&sample(NetworkId(11_155_111)))
            .await
            .unwrap();
        sessions.store(&sample(NetworkId(1337))).await.unwrap();
        assert_eq!(sessions.load().await.unwrap().network, NetworkId(1337));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_dir_is_under_xdg_runtime_dir() {
        assert_eq!(
            runtime_dir(Some("/run/user/501".into())),
            Some(PathBuf::from("/run/user/501/edw"))
        );
    }

    #[test]
    fn runtime_dir_is_none_without_xdg_runtime_dir() {
        assert_eq!(runtime_dir(None), None);
    }

    #[test]
    fn runtime_dir_treats_an_empty_variable_as_unset() {
        assert_eq!(runtime_dir(Some("".into())), None);
    }

    #[tokio::test]
    async fn clear_reports_a_store_that_cannot_be_opened() {
        let sessions = SessionFile(Some(Err("the session store cannot be opened".into())));
        assert!(sessions.clear().await.is_err());
    }

    #[tokio::test]
    async fn clear_reports_whether_a_session_existed() {
        let (sessions, dir) = isolated();
        sessions.store(&sample(NetworkId(1))).await.unwrap();
        assert!(sessions.clear().await.unwrap());
        assert!(!sessions.clear().await.unwrap());
        assert!(sessions.load().await.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_session_for_a_removed_instance_is_dropped() {
        let (sessions, runtime) = isolated();
        sessions.store(&sample(NetworkId(1))).await.unwrap();

        assert!(sessions.load_if_present().await.is_none());
        assert!(sessions.load().await.is_none());
        let _ = std::fs::remove_dir_all(runtime);
    }

    #[tokio::test]
    async fn a_session_for_an_existing_instance_is_kept() {
        let (sessions, runtime) = isolated();
        let data = std::env::temp_dir().join(format!(
            "edw-data-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let instance = data.join("mainnet");
        std::fs::create_dir_all(&instance).unwrap();
        std::fs::write(instance.join("record"), b"x").unwrap();
        let session = Session {
            data_dir: DataDir::new(&data).path().to_path_buf(),
            network: NetworkId(1),
            password: Zeroizing::new("secret".into()),
        };
        sessions.store(&session).await.unwrap();

        let loaded = sessions.load_if_present().await.unwrap();
        assert_eq!(loaded.network, NetworkId(1));
        assert_eq!(loaded.password.as_str(), "secret");
        let _ = std::fs::remove_dir_all(runtime);
        let _ = std::fs::remove_dir_all(data);
    }
}
