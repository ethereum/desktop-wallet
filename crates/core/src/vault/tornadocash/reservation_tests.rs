//! Reservation safety across independent vaults, failures, restarts, and CLI processes.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use alloy_signer_local::PrivateKeySigner;
use alloy_transport::mock::Asserter;
use kohaku_tornadocash::{
    Pool,
    syncer::{DynSyncer, Snapshot, Syncer, SyncerError},
};
use zeroize::Zeroizing;

use super::{
    TornadoVault, TornadoVaultError,
    db::{PoolDb, TornadoVaultDb},
};
use crate::{
    database::{Database, DatabaseError, file::FileDatabase, memory::MemoryDatabase},
    test_support::{TempDir, mocked_provider},
};

struct FailNoteWrite {
    inner: MemoryDatabase,
    fail: AtomicBool,
}
struct EmptySyncer;
struct PausedSyncer {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

struct BlockingSigner {
    started: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl Syncer for PausedSyncer {
    async fn sync_range(&self, pool: &Pool, from: u64, to: u64) -> Result<Snapshot, SyncerError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(Snapshot::new(pool.clone(), from..to, Vec::new()))
    }
}

#[async_trait::async_trait]
impl alloy_signer::Signer for BlockingSigner {
    async fn sign_hash(
        &self,
        _hash: &alloy_primitives::B256,
    ) -> alloy_signer::Result<alloy_primitives::Signature> {
        self.started.notify_one();
        std::future::pending().await
    }
    fn address(&self) -> alloy_primitives::Address {
        alloy_primitives::Address::repeat_byte(0x42)
    }
    fn chain_id(&self) -> Option<u64> {
        None
    }
    fn set_chain_id(&mut self, _chain: Option<u64>) {}
}

#[async_trait::async_trait]
impl Database for FailNoteWrite {
    async fn get(&self, key: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, DatabaseError> {
        self.inner.get(key).await
    }
    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), DatabaseError> {
        if key.ends_with(b"notes") && self.fail.swap(false, Ordering::SeqCst) {
            return Err(DatabaseError::Other("injected note write failure".into()));
        }
        self.inner.put(key, value).await
    }
    async fn delete(&self, key: &[u8]) -> Result<(), DatabaseError> {
        self.inner.delete(key).await
    }
}

#[async_trait::async_trait]
impl Syncer for EmptySyncer {
    async fn sync_range(&self, pool: &Pool, from: u64, to: u64) -> Result<Snapshot, SyncerError> {
        Ok(Snapshot::new(pool.clone(), from..to, Vec::new()))
    }
}

fn vault(db: Arc<dyn Database>) -> TornadoVault<PrivateKeySigner> {
    TornadoVault::new(
        PrivateKeySigner::from_slice(&[1; 32]).unwrap(),
        db,
        mocked_provider(&Asserter::new()),
    )
}

#[tokio::test]
async fn concurrent_vaults_reserve_distinct_nonces_across_pools() {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::default());
    let first = vault(db.clone());
    let second = vault(db.clone());
    let (one, two) = tokio::join!(
        first.prepare_deposits(&[Pool::ETHEREUM_ETHER_01]),
        second.prepare_deposits(&[Pool::ETHEREUM_USDC_100])
    );
    let one = one.unwrap();
    let two = two.unwrap();
    assert_ne!(one[0].nonce, two[0].nonce);
    assert_ne!(one[0].commitment, two[0].commitment);
    assert_eq!(db.next_nonce().await.unwrap(), 2);
    assert_eq!(two[0].calls.len(), 2, "token approval precedes deposit");
}

#[tokio::test]
async fn failed_note_persistence_burns_nonce_without_exposing_calls() {
    let db = Arc::new(FailNoteWrite {
        inner: MemoryDatabase::default(),
        fail: AtomicBool::new(true),
    });
    let first = vault(db.clone());
    assert!(
        first
            .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
            .await
            .is_err()
    );
    assert_eq!(db.next_nonce().await.unwrap(), 1);
    let next = vault(db.clone())
        .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
        .await
        .unwrap();
    assert_eq!(next[0].nonce, 1);
    assert_eq!(db.next_nonce().await.unwrap(), 2);
}

#[tokio::test]
async fn cancellation_after_reservation_does_not_recycle_nonce_or_hold_lock() {
    let db = Arc::new(MemoryDatabase::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let blocking = TornadoVault::new(
        BlockingSigner {
            started: started.clone(),
        },
        db.clone(),
        mocked_provider(&Asserter::new()),
    );
    let task =
        tokio::spawn(async move { blocking.prepare_deposits(&[Pool::ETHEREUM_ETHER_01]).await });
    started.notified().await;
    assert_eq!(db.next_nonce().await.unwrap(), 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let next = vault(db)
        .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
        .await
        .unwrap();
    assert_eq!(next[0].nonce, 1);
}

#[tokio::test]
async fn nonce_exhaustion_fails_without_wrapping() {
    let db = Arc::new(MemoryDatabase::default());
    db.put_next_nonce(u64::MAX).await.unwrap();
    assert!(matches!(
        vault(db.clone())
            .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
            .await,
        Err(TornadoVaultError::NonceExhausted)
    ));
    assert_eq!(db.next_nonce().await.unwrap(), u64::MAX);
}

#[tokio::test]
async fn recovery_preserves_pending_reservations_and_never_rewinds_nonce() {
    let db = Arc::new(MemoryDatabase::default());
    let asserter = Asserter::new();
    asserter.push_success(&alloy_primitives::U64::from(30_000_000));
    let vault = TornadoVault::with_syncer(
        PrivateKeySigner::from_slice(&[1; 32]).unwrap(),
        db.clone(),
        mocked_provider(&asserter),
        DynSyncer::new(EmptySyncer),
    );
    let pool = Pool::ETHEREUM_USDC_100;
    let prepared = vault
        .prepare_deposits(&[pool.clone(), pool.clone()])
        .await
        .unwrap();
    vault.recover(std::slice::from_ref(&pool)).await.unwrap();
    let notes = vault.pool_db(&pool).notes().await.unwrap();
    assert_eq!(notes.len(), 2);
    assert!(notes.iter().all(|note| note.leaf_index.is_none()));
    assert_eq!(db.next_nonce().await.unwrap(), 2);
    let later = vault.prepare_deposits(&[pool]).await.unwrap();
    assert_eq!(later[0].nonce, 2);
    assert!(
        prepared
            .iter()
            .all(|note| note.commitment != later[0].commitment)
    );
}

#[tokio::test]
async fn balance_sync_holds_the_same_lock_as_preparation() {
    use crate::{asset::AssetId, vault::Vault};
    let db = Arc::new(MemoryDatabase::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let asserter = Asserter::new();
    asserter.push_success(&alloy_primitives::U64::from(1));
    asserter.push_success(&alloy_primitives::U64::from(30_000_000));
    let reader = TornadoVault::with_syncer(
        PrivateKeySigner::from_slice(&[1; 32]).unwrap(),
        db.clone(),
        mocked_provider(&asserter),
        DynSyncer::new(PausedSyncer {
            started: started.clone(),
            release: release.clone(),
        }),
    );
    reader
        .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
        .await
        .unwrap();
    let task = tokio::spawn(async move { reader.balance(&AssetId::Native).await });
    started.notified().await;
    let writer = vault(db.clone());
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            writer.prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
        )
        .await
        .is_err()
    );
    assert_eq!(db.next_nonce().await.unwrap(), 1);
    release.notify_one();
    task.await.unwrap().unwrap();
    let prepared = writer
        .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
        .await
        .unwrap();
    assert_eq!(prepared[0].nonce, 1);
}

#[tokio::test]
async fn restart_reads_the_durable_high_water_mark() {
    let dir = TempDir::new();
    let pool = Pool::ETHEREUM_ETHER_01;
    let first = vault(Arc::new(FileDatabase::open(dir.path()).unwrap()))
        .prepare_deposits(std::slice::from_ref(&pool))
        .await
        .unwrap();
    let second = vault(Arc::new(FileDatabase::open(dir.path()).unwrap()))
        .prepare_deposits(&[pool])
        .await
        .unwrap();
    assert_eq!(second[0].nonce, first[0].nonce + 1);
    assert_ne!(first[0].commitment, second[0].commitment);
}

#[test]
fn process_worker() {
    let Some(path) = std::env::var_os("EDW_RESERVATION_TEST_DB") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let vault = vault(Arc::new(FileDatabase::open(path).unwrap()));
        for _ in 0..16 {
            vault
                .prepare_deposits(&[Pool::ETHEREUM_ETHER_01])
                .await
                .unwrap();
        }
    });
}

#[tokio::test]
async fn separate_processes_do_not_reuse_nonces_or_overwrite_pending_notes() {
    let dir = TempDir::new();
    let exe = std::env::current_exe().unwrap();
    let mut first = tokio::process::Command::new(&exe)
        .args([
            "--exact",
            "vault::tornadocash::reservation_tests::process_worker",
        ])
        .env("EDW_RESERVATION_TEST_DB", dir.path())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut second = tokio::process::Command::new(&exe)
        .args([
            "--exact",
            "vault::tornadocash::reservation_tests::process_worker",
        ])
        .env("EDW_RESERVATION_TEST_DB", dir.path())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let (one, two) = tokio::join!(first.wait(), second.wait());
    assert!(one.unwrap().success());
    assert!(two.unwrap().success());
    let db = Arc::new(FileDatabase::open(dir.path()).unwrap());
    assert_eq!(db.next_nonce().await.unwrap(), 32);
    let notes = vault(db)
        .pool_db(&Pool::ETHEREUM_ETHER_01)
        .notes()
        .await
        .unwrap();
    assert_eq!(notes.len(), 32);
    assert_eq!(
        notes
            .iter()
            .map(|note| note.commitment)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        32
    );
    assert_eq!(
        notes
            .iter()
            .map(|note| note.nonce)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        32
    );
}
