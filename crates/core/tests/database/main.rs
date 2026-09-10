//! Public-API tests for [`EncryptedDatabase`].
#![allow(clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use edw_core::database::{
    Database,
    encrypted::{EncryptedDatabase, EncryptedDatabaseError},
    file::FileDatabase,
    memory::MemoryDatabase,
    scoped::ScopedDatabaseExt,
};
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery staple";
const NEXT_PASSWORD: &[u8] = b"a different passphrase entirely";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("edw-test-{}", Uuid::new_v4())))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn memory() -> Arc<dyn Database> {
    Arc::new(MemoryDatabase::new())
}

fn file(dir: &Path) -> Arc<dyn Database> {
    Arc::new(FileDatabase::open(dir).expect("open file database"))
}

#[tokio::test]
async fn records_round_trip() {
    let store = EncryptedDatabase::create(memory(), PASSWORD)
        .await
        .expect("create");

    store.put(b"greeting", b"hello").await.expect("put");
    let value = store.get(b"greeting").await.expect("get").expect("present");
    assert_eq!(&*value, b"hello");
    assert!(store.get(b"absent").await.expect("get").is_none());

    store.delete(b"greeting").await.expect("delete");
    assert!(store.get(b"greeting").await.expect("get").is_none());
}

#[tokio::test]
async fn records_survive_unlock() {
    let dir = TempDir::new();

    let store = EncryptedDatabase::create(file(dir.path()), PASSWORD)
        .await
        .expect("create");
    store.put(b"greeting", b"hello").await.expect("put");
    drop(store);

    let store = EncryptedDatabase::unlock(file(dir.path()), PASSWORD)
        .await
        .expect("unlock");
    let value = store.get(b"greeting").await.expect("get").expect("present");
    assert_eq!(&*value, b"hello");
}

#[tokio::test]
async fn wrong_password_is_rejected() {
    let dir = TempDir::new();

    EncryptedDatabase::create(file(dir.path()), PASSWORD)
        .await
        .expect("create");

    let err = EncryptedDatabase::unlock(file(dir.path()), b"wrong password")
        .await
        .err()
        .expect("unlock must fail");
    assert!(matches!(err, EncryptedDatabaseError::InvalidPassword));
}

#[tokio::test]
async fn unlock_requires_an_initialized_store() {
    let err = EncryptedDatabase::unlock(memory(), PASSWORD)
        .await
        .err()
        .expect("unlock must fail");
    assert!(matches!(err, EncryptedDatabaseError::NotInitialized));
}

#[tokio::test]
async fn create_refuses_an_existing_store() {
    let backend = memory();
    EncryptedDatabase::create(backend.clone(), PASSWORD)
        .await
        .expect("create");

    let err = EncryptedDatabase::create(backend, b"another password")
        .await
        .err()
        .expect("second create must fail");
    assert!(matches!(err, EncryptedDatabaseError::AlreadyInitialized));
}

#[tokio::test]
async fn create_rejects_an_empty_password() {
    let err = EncryptedDatabase::create(memory(), b"")
        .await
        .err()
        .expect("create must fail");
    assert!(matches!(err, EncryptedDatabaseError::EmptyPassword));
}

#[tokio::test]
async fn unlock_rejects_an_empty_password() {
    let backend = memory();
    EncryptedDatabase::create(backend.clone(), PASSWORD)
        .await
        .expect("create");

    let err = EncryptedDatabase::unlock(backend, b"")
        .await
        .err()
        .expect("unlock must fail");
    assert!(matches!(err, EncryptedDatabaseError::EmptyPassword));
}

#[tokio::test]
async fn scoped_records_are_isolated() {
    let store: Arc<dyn Database> = Arc::new(
        EncryptedDatabase::create(memory(), PASSWORD)
            .await
            .expect("create"),
    );
    let first = store.clone().scoped(Uuid::new_v4().as_bytes());
    let second = store.clone().scoped(Uuid::new_v4().as_bytes());

    first.put(b"pk", b"first secret").await.expect("put");
    second.put(b"pk", b"second secret").await.expect("put");

    assert_eq!(
        &*first.get(b"pk").await.expect("get").expect("present"),
        b"first secret"
    );
    assert_eq!(
        &*second.get(b"pk").await.expect("get").expect("present"),
        b"second secret"
    );
    assert!(store.get(b"pk").await.expect("get").is_none());
}

#[tokio::test]
async fn rewriting_one_record_does_not_lose_another() {
    let store = EncryptedDatabase::create(memory(), PASSWORD)
        .await
        .expect("create");
    store.put(b"first", b"one").await.expect("put");
    store.put(b"second", b"two").await.expect("put");

    store.put(b"first", b"one again").await.expect("put");

    assert_eq!(
        &*store.get(b"first").await.expect("get").expect("present"),
        b"one again"
    );
    assert_eq!(
        &*store.get(b"second").await.expect("get").expect("present"),
        b"two"
    );
}

#[tokio::test]
async fn change_password_round_trips_through_unlock() {
    let dir = TempDir::new();

    let store = EncryptedDatabase::create(file(dir.path()), PASSWORD)
        .await
        .expect("create");
    store.put(b"pk", b"secret").await.expect("put");
    store
        .change_password(NEXT_PASSWORD)
        .await
        .expect("change password");
    drop(store);

    let store = EncryptedDatabase::unlock(file(dir.path()), NEXT_PASSWORD)
        .await
        .expect("unlock");
    assert_eq!(
        &*store.get(b"pk").await.expect("get").expect("present"),
        b"secret"
    );
}
