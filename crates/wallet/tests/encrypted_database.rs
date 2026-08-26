//! Tests for the encryption-at-rest seam (EDW-003).
//!
//! `clippy.toml` allows `expect` inside test functions; these module-level helpers sit
//! outside one, and a failure in them is still just a failed test.
#![allow(clippy::expect_used)]

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use alloy_primitives::hex;
use edw_core::database::Database;
use edw_wallet::database::{
    encrypted::{EncryptedDatabase, EncryptedDatabaseError},
    file::FileDatabase,
    memory::MemoryDatabase,
    scoped::ScopedDatabaseExt,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery staple";
const HEADER_KEY: &[u8] = b"edw:keystore:v1";
/// Byte offset of the `t_cost` varint: 8 magic, 1 version, 3 for the `m_cost` varint.
const T_COST_OFFSET: usize = 12;

/// Removes its directory on drop, so a failing assertion does not leave a store behind.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("edw-test-{}", Uuid::new_v4())))
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn memory_backend() -> Arc<MemoryDatabase> {
    Arc::new(MemoryDatabase::new())
}

fn file_backend(dir: &Path) -> Arc<dyn Database> {
    Arc::new(FileDatabase::open(dir).expect("open file database"))
}

fn keys_of(backend: &Arc<MemoryDatabase>) -> HashSet<Vec<u8>> {
    backend.keys().expect("keys").into_iter().collect()
}

#[tokio::test]
async fn records_round_trip_through_encryption() {
    let store = EncryptedDatabase::create(memory_backend(), PASSWORD)
        .await
        .expect("create");

    store.put(b"greeting", b"hello").await.expect("put");
    let value = store.get(b"greeting").await.expect("get").expect("present");

    assert_eq!(value.as_slice(), b"hello");
    assert!(store.get(b"absent").await.expect("get").is_none());

    store.delete(b"greeting").await.expect("delete");
    assert!(store.get(b"greeting").await.expect("get").is_none());
}

#[tokio::test]
async fn records_survive_a_lock_and_unlock_cycle() {
    let dir = TempDir::new();
    let path = dir.join("store");

    let store = EncryptedDatabase::create(file_backend(&path), PASSWORD)
        .await
        .expect("create");
    store.put(b"greeting", b"hello").await.expect("put");
    drop(store);

    let store = EncryptedDatabase::unlock(file_backend(&path), PASSWORD)
        .await
        .expect("unlock");
    let value = store.get(b"greeting").await.expect("get").expect("present");
    assert_eq!(value.as_slice(), b"hello");
}

/// EDW-003: a wrong password fails via the AEAD tag, cleanly, not via a panic.
#[tokio::test]
async fn wrong_password_is_rejected_cleanly() {
    let dir = TempDir::new();
    let path = dir.join("store");

    let store = EncryptedDatabase::create(file_backend(&path), PASSWORD)
        .await
        .expect("create");
    store.put(b"pk", b"secret").await.expect("put");
    drop(store);

    let err = EncryptedDatabase::unlock(file_backend(&path), b"wrong password")
        .await
        .err()
        .expect("unlock must fail");

    assert!(
        matches!(err, EncryptedDatabaseError::InvalidPassword),
        "expected InvalidPassword, got {err:?}"
    );
}

#[tokio::test]
async fn unlocking_an_uninitialized_store_fails() {
    let err = EncryptedDatabase::unlock(memory_backend(), PASSWORD)
        .await
        .err()
        .expect("unlock must fail");

    assert!(matches!(err, EncryptedDatabaseError::NotInitialized));
}

#[tokio::test]
async fn create_refuses_to_rekey_an_existing_store() {
    let backend = memory_backend();
    let store = EncryptedDatabase::create(backend.clone(), PASSWORD)
        .await
        .expect("create");
    store.put(b"pk", b"secret").await.expect("put");

    let err = EncryptedDatabase::create(backend, b"another password")
        .await
        .err()
        .expect("second create must fail");

    assert!(matches!(err, EncryptedDatabaseError::AlreadyInitialized));
}

/// EDW-003: per-object scoping gives each vault and executor an isolated keyspace. The
/// isolation is cryptographic rather than a bare key prefix, so a blob lifted out of one
/// scope will not open in another even when written directly into its storage slot.
#[tokio::test]
async fn scopes_are_cryptographically_isolated() {
    let backend = memory_backend();
    let store: Arc<dyn Database> = Arc::new(
        EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .expect("create"),
    );

    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let first = store.clone().scoped(first_id.as_bytes());
    let second = store.clone().scoped(second_id.as_bytes());

    let before = keys_of(&backend);
    first.put(b"pk", b"first secret").await.expect("put");
    let after_first = keys_of(&backend);
    second.put(b"pk", b"second secret").await.expect("put");
    let after_second = keys_of(&backend);

    let first_slot = sole_new_key(&before, &after_first);
    let second_slot = sole_new_key(&after_first, &after_second);
    assert_ne!(first_slot, second_slot, "scopes shared a storage slot");

    assert_eq!(
        first
            .get(b"pk")
            .await
            .expect("get")
            .expect("present")
            .as_slice(),
        b"first secret"
    );
    assert!(
        store.get(b"pk").await.expect("get").is_none(),
        "an unscoped read must not reach a scoped record"
    );

    let lifted = backend
        .get(&first_slot)
        .await
        .expect("get")
        .expect("present")
        .to_vec();
    backend.put(&second_slot, &lifted).await.expect("replay");

    assert!(
        second.get(b"pk").await.is_err(),
        "a record replayed into another scope must fail to decrypt"
    );
}

#[tokio::test]
async fn repeated_writes_of_one_value_produce_distinct_ciphertexts() {
    let backend = memory_backend();
    let store = EncryptedDatabase::create(backend.clone(), PASSWORD)
        .await
        .expect("create");

    let before = keys_of(&backend);
    store.put(b"pk", b"secret").await.expect("put");
    let slot = sole_new_key(&before, &keys_of(&backend));

    let first = backend
        .get(&slot)
        .await
        .expect("get")
        .expect("present")
        .to_vec();
    store.put(b"pk", b"secret").await.expect("put");
    let second = backend
        .get(&slot)
        .await
        .expect("get")
        .expect("present")
        .to_vec();

    assert_ne!(first, second, "nonce reuse: ciphertext repeated");
}

#[tokio::test]
async fn a_tampered_record_fails_to_open() {
    let backend = memory_backend();
    let store = EncryptedDatabase::create(backend.clone(), PASSWORD)
        .await
        .expect("create");

    let before = keys_of(&backend);
    store.put(b"pk", b"secret").await.expect("put");
    let slot = sole_new_key(&before, &keys_of(&backend));

    let mut blob = backend
        .get(&slot)
        .await
        .expect("get")
        .expect("present")
        .to_vec();
    let last = blob.len() - 1;
    blob[last] ^= 0x01;
    backend.put(&slot, &blob).await.expect("tamper");

    assert!(
        store.get(b"pk").await.is_err(),
        "a flipped tag byte must not decrypt"
    );
}

/// EDW-003: storage keys are blinded, so a backend never sees a logical key name and its
/// file discloses neither the key names nor how many vaults a profile holds.
#[tokio::test]
async fn storage_keys_are_blinded() {
    let backend = memory_backend();
    let store: Arc<dyn Database> = Arc::new(
        EncryptedDatabase::create(backend.clone(), PASSWORD)
            .await
            .expect("create"),
    );

    let vault_id = Uuid::new_v4();
    store.put(b"vaults", b"index").await.expect("put");
    store
        .clone()
        .scoped(vault_id.as_bytes())
        .put(b"pk", b"secret")
        .await
        .expect("put");

    for key in keys_of(&backend) {
        assert_ne!(key.as_slice(), b"pk", "logical key stored verbatim");
        assert_ne!(key.as_slice(), b"vaults", "logical key stored verbatim");
        assert!(
            !contains(&key, vault_id.as_bytes()),
            "scope id stored verbatim in a storage key"
        );
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// One file per key: a write is proportional to the record, not to the whole store, and a
/// torn write cannot reach a record other than the one being written.
#[tokio::test]
async fn each_record_gets_its_own_file() {
    let dir = TempDir::new();
    let path = dir.join("store");

    let store = EncryptedDatabase::create(file_backend(&path), PASSWORD)
        .await
        .expect("create");
    store.put(b"first", b"one").await.expect("put");
    store.put(b"second", b"two").await.expect("put");

    let files = record_files(&path);
    assert_eq!(files.len(), 3, "expected a header plus two records");

    let untouched = files
        .iter()
        .map(|p| std::fs::read(p).expect("read record"))
        .collect::<Vec<_>>();
    store.put(b"first", b"one again").await.expect("put");

    let after = record_files(&path)
        .iter()
        .map(|p| std::fs::read(p).expect("read record"))
        .collect::<Vec<_>>();
    let changed = untouched
        .iter()
        .filter(|bytes| !after.contains(bytes))
        .count();
    assert_eq!(changed, 1, "a write must rewrite exactly one record");

    assert_eq!(
        store
            .get(b"second")
            .await
            .expect("get")
            .expect("present")
            .as_slice(),
        b"two"
    );
}

fn record_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .expect("read store dir")
        .map(|e| e.expect("dir entry").path())
        .collect();
    paths.sort();
    paths
}

/// EDW-023: an empty password is rejected at this layer rather than silently accepted as a
/// root of trust.
#[tokio::test]
async fn empty_password_is_rejected() {
    let backend = memory_backend();
    let err = EncryptedDatabase::create(backend.clone(), b"")
        .await
        .err()
        .expect("create must reject an empty password");
    assert!(matches!(err, EncryptedDatabaseError::EmptyPassword));

    EncryptedDatabase::create(backend.clone(), PASSWORD)
        .await
        .expect("create");
    let err = EncryptedDatabase::unlock(backend, b"")
        .await
        .err()
        .expect("unlock must reject an empty password");
    assert!(matches!(err, EncryptedDatabaseError::EmptyPassword));
}

/// EDW-023: the header is plaintext and therefore untrusted input. Key-derivation parameters
/// taken from it are checked before use, so a corrupt or hostile header cannot drive the
/// process into an unbounded hash.
#[tokio::test]
async fn header_key_derivation_parameters_are_not_trusted() {
    let dir = TempDir::new();
    let path = dir.join("store");

    EncryptedDatabase::create(file_backend(&path), PASSWORD)
        .await
        .expect("create");

    // The header is stored unblinded under a known key, so its record is locatable.
    let header_path = path.join(hex::encode(Sha256::digest(HEADER_KEY)));
    let mut bytes = std::fs::read(&header_path).expect("read header");
    assert_eq!(&bytes[..8], b"EDWSTORE", "header layout changed");

    // postcard: magic[8] | version[1] | m_cost varint | t_cost | p_cost | salt | verifier.
    // Raising t_cost in place multiplies the work unlock would perform.
    assert_eq!(bytes[T_COST_OFFSET], 0x03, "t_cost not where expected");
    bytes[T_COST_OFFSET] = 0x1E;
    std::fs::write(&header_path, &bytes).expect("write header");

    let started = std::time::Instant::now();
    let err = EncryptedDatabase::unlock(file_backend(&path), PASSWORD)
        .await
        .err()
        .expect("unlock must reject unknown parameters");

    assert!(
        matches!(err, EncryptedDatabaseError::UnsupportedParameters),
        "expected UnsupportedParameters, got {err:?}"
    );
    // Rejection happens before any hashing. A generous bound still fails loudly if the check
    // is ever moved after key derivation, which takes over a second even at the honest cost.
    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "parameters were validated only after doing the work"
    );
}

/// EDW-023: a store is not readable by other local accounts. Encryption does not make a
/// world-readable file acceptable, since it is still an offline guessing target.
#[cfg(unix)]
#[tokio::test]
async fn store_is_not_world_readable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new();
    let path = dir.join("store");

    let store = EncryptedDatabase::create(file_backend(&path), PASSWORD)
        .await
        .expect("create");
    store.put(b"pk", b"secret").await.expect("put");

    let dir_mode = std::fs::metadata(&path)
        .expect("stat dir")
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700, "store directory is too permissive");

    for record in record_files(&path) {
        let mode = std::fs::metadata(&record)
            .expect("stat record")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "record {record:?} is too permissive");
    }
}

/// A directory left permissive by an earlier build or a loose umask is tightened on open.
#[cfg(unix)]
#[tokio::test]
async fn an_existing_permissive_directory_is_tightened() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new();
    let path = dir.join("store");
    std::fs::create_dir_all(&path).expect("create dir");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("loosen");

    let _ = file_backend(&path);

    let mode = std::fs::metadata(&path)
        .expect("stat dir")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700, "existing directory was not tightened");
}

fn sole_new_key(before: &HashSet<Vec<u8>>, after: &HashSet<Vec<u8>>) -> Vec<u8> {
    let mut added = after.difference(before);
    let key = added.next().expect("a write should add one key").clone();
    assert!(added.next().is_none(), "a write added more than one key");
    key
}
