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
    /// # Errors
    /// Returns an error if the directory cannot be created.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, FileDatabaseError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
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
        // contents are still buffered.
        let write = || -> Result<(), std::io::Error> {
            let file = std::fs::File::create(&tmp)?;
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
