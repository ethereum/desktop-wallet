use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use edw_core::{
    database::{Database, file::FileDatabase},
    network::NetworkId,
};
use zeroize::Zeroizing;

const TTL: Duration = Duration::from_mins(15);

const RECORD: &[u8] = b"session";

/// The unlocked network and its password, kept between commands for [`TTL`] after last use.
pub struct Session {
    pub data_dir: PathBuf,
    pub network: NetworkId,
    pub password: Zeroizing<String>,
}

/// Where the session is kept: a private store under `$XDG_RUNTIME_DIR/edw`. Without a
/// runtime dir nothing can be kept, so every command finds the wallet locked.
pub struct SessionFile(Option<FileDatabase>);

impl SessionFile {
    pub fn runtime() -> Self {
        Self(
            std::env::var_os("XDG_RUNTIME_DIR")
                .and_then(|dir| FileDatabase::open(PathBuf::from(dir).join("edw")).ok()),
        )
    }

    /// The live session, renewed for another [`TTL`]. An expired one is removed.
    pub async fn load(&self) -> Option<Session> {
        let store = self.0.as_ref()?;
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

    pub async fn store(&self, session: &Session) -> Result<(), anyhow::Error> {
        let store = self
            .0
            .as_ref()
            .context("XDG_RUNTIME_DIR is not set, so no session can be held")?;
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
        let Some(store) = &self.0 else {
            return Ok(false);
        };
        let existed = store.get(RECORD).await?.is_some();
        store.delete(RECORD).await?;
        Ok(existed)
    }
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
        (SessionFile(Some(FileDatabase::open(&dir).unwrap())), dir)
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

    #[tokio::test]
    async fn clear_reports_whether_a_session_existed() {
        let (sessions, dir) = isolated();
        sessions.store(&sample(NetworkId(1))).await.unwrap();
        assert!(sessions.clear().await.unwrap());
        assert!(!sessions.clear().await.unwrap());
        assert!(sessions.load().await.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
