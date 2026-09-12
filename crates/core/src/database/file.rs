use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use alloy_primitives::hex;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{Database, DatabaseError};

/// Directory-backed [`Database`] with one file per record.
///
/// On Unix the directory is `0700` and each record `0600`. This is not encryption: wrap it
/// in [`super::encrypted::EncryptedDatabase`] before storing anything sensitive.
pub struct FileDatabase {
    dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum FileDatabaseError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl FileDatabase {
    /// Opens the store rooted at `dir`, creating it if needed. An existing directory has its
    /// permissions tightened rather than being rejected.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, FileDatabaseError> {
        let dir = dir.as_ref().to_path_buf();
        create_private_dir(&dir)?;
        Ok(Self { dir })
    }

    /// Hash of the key as a filename. Not confidentiality: blinding happens in
    /// [`super::encrypted::EncryptedDatabase`].
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

        // Flush before rename so a published file is complete; set the mode at creation so
        // the record is never briefly world-readable.
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

impl From<FileDatabaseError> for DatabaseError {
    fn from(err: FileDatabaseError) -> Self {
        DatabaseError::Other(Box::new(err))
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
//? Windows target needs its own handling before it stores anything sensitive.
#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dir)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<std::fs::File, std::io::Error> {
    std::fs::File::create(path)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::test_support::TempDir;

    fn record_files(dir: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        paths
    }

    #[tokio::test]
    async fn put_writes_one_file_per_key() {
        let dir = TempDir::new();
        let db = FileDatabase::open(dir.path()).unwrap();
        db.put(b"first", b"one").await.unwrap();
        db.put(b"second", b"two").await.unwrap();
        assert_eq!(record_files(dir.path()).len(), 2);
    }

    #[tokio::test]
    async fn rewriting_one_key_does_not_change_other_files() {
        let dir = TempDir::new();
        let db = FileDatabase::open(dir.path()).unwrap();
        db.put(b"first", b"one").await.unwrap();
        db.put(b"second", b"two").await.unwrap();

        let before: Vec<_> = record_files(dir.path())
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();
        db.put(b"first", b"one again").await.unwrap();
        let after: Vec<_> = record_files(dir.path())
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();

        let changed = before.iter().filter(|bytes| !after.contains(bytes)).count();
        assert_eq!(changed, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn new_store_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let db = FileDatabase::open(dir.path()).unwrap();
        db.put(b"pk", b"secret").await.unwrap();

        let dir_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);

        for record in record_files(dir.path()) {
            let mode = std::fs::metadata(&record).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{record:?}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_permissive_directory_is_tightened() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        FileDatabase::open(dir.path()).unwrap();

        let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
