//! EDW-003: a profile written through the encrypted seam leaves no plaintext key material
//! in the file on disk.
//!
//! This lives inside the crate rather than in `tests/` because it drives the real repository
//! traits, and those stay crate-private: `get_signing_key` returns secret material, so it
//! must not be reachable from the crate's public API.

use std::{path::PathBuf, sync::Arc};

use alloy_primitives::{Address, hex};
use alloy_signer_local::PrivateKeySigner;
use uuid::Uuid;

use crate::{
    database::{
        Database, encrypted::EncryptedDatabase, file::FileDatabase, scoped::ScopedDatabaseExt,
    },
    executor::simple::db::SimpleExecutorDb,
    profile::simple::db::SimpleProfileDb,
    vault::simple::db::SimpleVaultDb,
};

const PASSWORD: &[u8] = b"correct horse battery staple";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("edw-test-{}", Uuid::new_v4())))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn backend(dir: &std::path::Path) -> Arc<dyn Database> {
    Arc::new(FileDatabase::open(dir).expect("open file database"))
}

/// Every byte the store put on disk: each record's filename as well as its contents, so
/// that a key name leaking into a filename fails the scan too.
fn on_disk_bytes(dir: &std::path::Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read store dir") {
        let entry = entry.expect("dir entry");
        bytes.extend_from_slice(entry.file_name().as_encoded_bytes());
        bytes.extend_from_slice(&std::fs::read(entry.path()).expect("read record"));
    }
    bytes
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test]
async fn no_plaintext_key_material_on_disk() {
    let dir = TempDir::new();
    let path = dir.0.join("store");

    let signer = PrivateKeySigner::random();
    let signing_key = signer.credential().clone();
    let secret = signing_key.to_bytes().to_vec();
    let address = signer.address();
    let implementation = Address::repeat_byte(0xAB);

    let vault_id = Uuid::new_v4();
    let executor_id = Uuid::new_v4();

    {
        let store: Arc<dyn Database> = Arc::new(
            EncryptedDatabase::create(backend(&path), PASSWORD)
                .await
                .expect("create"),
        );

        let vault = store.clone().scoped(vault_id.as_bytes());
        SimpleVaultDb::put_signing_key(&vault, &signing_key)
            .await
            .expect("vault key");
        SimpleVaultDb::put_implementation(&vault, &implementation)
            .await
            .expect("vault implementation");

        let executor = store.clone().scoped(executor_id.as_bytes());
        SimpleExecutorDb::put_signing_key(&executor, &signing_key)
            .await
            .expect("executor key");

        store
            .put_vaults(&[(vault_id, "simple-vault")])
            .await
            .expect("vault index");
        store
            .put_executor((executor_id, "simple-executor"))
            .await
            .expect("executor index");
    }

    let bytes = on_disk_bytes(&path);
    assert!(!bytes.is_empty(), "store should not be empty");

    let secret_hex = hex::encode(&secret);
    let forbidden: [(&str, Vec<u8>); 7] = [
        ("private key", secret.clone()),
        (
            "private key, lowercase hex",
            secret_hex.clone().into_bytes(),
        ),
        (
            "private key, uppercase hex",
            secret_hex.to_uppercase().into_bytes(),
        ),
        ("vault address", address.to_vec()),
        ("implementation address", implementation.to_vec()),
        ("vault id", vault_id.as_bytes().to_vec()),
        ("executor id", executor_id.as_bytes().to_vec()),
    ];
    for (what, needle) in &forbidden {
        assert!(
            !contains(&bytes, needle),
            "{what} found in plaintext on disk"
        );
    }

    // Logical key names are blinded too, so the file does not disclose the store's shape.
    // Only names long enough that a chance hit in ciphertext is negligible are scanned for;
    // `encrypted_database::storage_keys_are_blinded` covers blinding exactly.
    for name in [
        b"implementation".as_slice(),
        b"vaults".as_slice(),
        b"executor".as_slice(),
        b"simple-vault".as_slice(),
        b"simple-executor".as_slice(),
    ] {
        assert!(
            !contains(&bytes, name),
            "logical key {:?} found in plaintext on disk",
            String::from_utf8_lossy(name)
        );
    }

    let store: Arc<dyn Database> = Arc::new(
        EncryptedDatabase::unlock(backend(&path), PASSWORD)
            .await
            .expect("unlock"),
    );
    let vault = store.clone().scoped(vault_id.as_bytes());
    assert_eq!(
        SimpleVaultDb::get_signing_key(&vault)
            .await
            .expect("recover key")
            .to_bytes()
            .to_vec(),
        secret
    );
    assert_eq!(
        SimpleVaultDb::get_implementation(&vault)
            .await
            .expect("recover implementation"),
        implementation
    );
    assert_eq!(
        store.get_vaults().await.expect("recover vaults"),
        vec![(vault_id, "simple-vault".to_string())]
    );
}
