use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use edw_core::database::{Database, encrypted::EncryptedDatabase, file::FileDatabase};

use crate::unlock;

pub(crate) fn network_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("network")
}

/// Opens the network store, creating it under the wallet password if it does not exist.
pub(crate) async fn network_store(data_dir: &Path) -> Result<Arc<dyn Database>, anyhow::Error> {
    encrypted_store(data_dir, &network_dir(data_dir)).await
}

/// Opens the network store only if it is already initialized.
pub(crate) async fn try_network_store(
    data_dir: &Path,
) -> Result<Option<Arc<dyn Database>>, anyhow::Error> {
    let dir = network_dir(data_dir);
    if !unlock::is_initialized(&dir) {
        return Ok(None);
    }
    Ok(Some(encrypted_store(data_dir, &dir).await?))
}

pub(crate) async fn profile_store(
    name: &str,
    data_dir: &Path,
) -> Result<Arc<dyn Database>, anyhow::Error> {
    encrypted_store(data_dir, &data_dir.join(name).join("db")).await
}

/// Unlocks `dir`, or creates a store if it is empty. The caller supplies the already-verified
/// wallet password.
pub(crate) async fn open_store(
    dir: &Path,
    password: &[u8],
) -> Result<Arc<dyn Database>, anyhow::Error> {
    let initialized = unlock::is_initialized(dir);
    let backend: Arc<dyn Database> = Arc::new(
        FileDatabase::open(dir)
            .with_context(|| format!("error opening the store at {}", dir.display()))?,
    );
    let store = if initialized {
        EncryptedDatabase::unlock(backend, password)
            .await
            .context("error unlocking the store")?
    } else {
        EncryptedDatabase::create(backend, password)
            .await
            .context("error creating the store")?
    };
    Ok(Arc::new(store))
}

async fn encrypted_store(data_dir: &Path, dir: &Path) -> Result<Arc<dyn Database>, anyhow::Error> {
    let password = unlock::ensure_unlocked(data_dir).await?;
    open_store(dir, password.as_bytes()).await
}
