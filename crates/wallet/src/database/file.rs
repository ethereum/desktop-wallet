use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use alloy_primitives::hex;
use edw_core::database::{Database, DatabaseError};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// A directory-backed [`Database`], storing each record in its own file.
///
/// One file per key keeps a write proportional to the record rather than to the whole store,
/// and keeps a torn write confined to the record being written.
///
/// On Unix the directory is created `0700` and each record `0600`, matching the convention
/// established by Geth and Bitcoin Core. Encryption does not make a world-readable store
/// acceptable: the ciphertext is still an offline password-guessing target, so the fewer
/// local accounts that can copy it, the better.
///
/// This secures nothing on its own: wrap it in [`super::encrypted::EncryptedDatabase`] before
/// storing anything sensitive.
pub struct FileDatabase {
    dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum FileDatabaseError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl FileDatabase {
    /// Opens the store rooted at `dir`, creating the directory if it does not exist.
    ///
    /// An existing directory has its permissions tightened rather than being rejected, since a
    /// store left readable by a permissive umask is a defect to repair, not a reason to refuse
    /// to start.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created or its permissions cannot be set.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, FileDatabaseError> {
        let dir = dir.as_ref().to_path_buf();
        create_private_dir(&dir)?;
        Ok(Self { dir })
    }

    /// Names a record's file by the hash of its key, so that a key of any length or byte
    /// content maps to a valid fixed-length filename.
    ///
    /// This is not confidentiality: a hash of a guessable key is guessable. Key privacy comes
    /// from [`super::encrypted::EncryptedDatabase`], which blinds keys before they reach a
    /// backend.
    fn key_path(&self, key: &[u8]) -> PathBuf {
        let digest = Sha256::digest(key);
        self.dir.join(hex::encode(digest))
    }
}

//? File I/O runs inline on the caller's task. Records are small and writes are rare, but a
//? high-traffic backend should move this off the async executor.
#[async_trait::async_trait]
impl Database for FileDatabase {
    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError> {
        match std::fs::read(self.key_path(key)) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(FileDatabaseError::Io(e).into()),
        }
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        let path = self.key_path(key);
        let tmp = path.with_extension("tmp");

        // Flush the replacement before publishing it, so the rename cannot expose a file whose
        // contents are still buffered. The mode is set at creation rather than after, so the
        // record is never briefly world-readable.
        let write = || -> Result<(), std::io::Error> {
            let file = create_private_file(&tmp)?;
            std::io::Write::write_all(&mut &file, value)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &path)
        };

        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            DatabaseError::from(FileDatabaseError::Io(e))
        })
    }

    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError> {
        match std::fs::remove_file(self.key_path(key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(FileDatabaseError::Io(e).into()),
        }
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        return Ok(());
    }

    let mut perms = std::fs::metadata(dir)?.permissions();
    if perms.mode() & 0o077 != 0 {
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms)?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<std::fs::File, std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

//? Windows ACLs are not a mode bitmask, so there is no equivalent one-line tightening. A
//? Windows target needs its own handling before it stores anything sensitive; see EDW-020.
#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dir)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<std::fs::File, std::io::Error> {
    std::fs::File::create(path)
}

impl From<FileDatabaseError> for DatabaseError {
    fn from(err: FileDatabaseError) -> Self {
        DatabaseError::Other(Box::new(err))
    }
}
