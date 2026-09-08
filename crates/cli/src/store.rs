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

/// Opens the network store only if it already has an encrypted header.
pub(crate) async fn try_network_store(
    data_dir: &Path,
) -> Result<Option<Arc<dyn Database>>, anyhow::Error> {
    let dir = network_dir(data_dir);
    if !header_at(&dir).await? {
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

async fn encrypted_store(data_dir: &Path, dir: &Path) -> Result<Arc<dyn Database>, anyhow::Error> {
    let password = unlock::ensure_unlocked(data_dir)?;
    let backend: Arc<dyn Database> = Arc::new(
        FileDatabase::open(dir)
            .with_context(|| format!("error opening the store at {}", dir.display()))?,
    );
    let store = EncryptedDatabase::open_or_create(backend, password.as_bytes())
        .await
        .context("error opening the store")?;
    Ok(Arc::new(store))
}

async fn header_at(dir: &Path) -> Result<bool, anyhow::Error> {
    if !dir.exists() {
        return Ok(false);
    }
    let backend =
        FileDatabase::open(dir).with_context(|| format!("error opening {}", dir.display()))?;
    Ok(EncryptedDatabase::has_header(&backend).await?)
}
