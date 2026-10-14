use kohaku_tornadocash::{Field, Pool, syncer::Snapshot};
use serde::{Deserialize, Serialize};

use crate::database::{Database, DatabaseError};

#[async_trait::async_trait]
pub trait TornadoVaultDb: Database {
    /// Returns the next unused keychain nonce, shared across every pool.
    async fn next_nonce(&self) -> Result<u64, TornadoVaultDbError> {
        let Some(bytes) = self.get(b"nextNonce").await? else {
            return Ok(0);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_next_nonce(&self, nonce: u64) -> Result<(), TornadoVaultDbError> {
        self.put(b"nextNonce", &postcard::to_stdvec(&nonce)?)
            .await?;
        Ok(())
    }

    /// Returns the pools the vault tracks. Only these are synced.
    async fn tracked_pools(&self) -> Result<Vec<Pool>, TornadoVaultDbError> {
        let Some(bytes) = self.get(b"trackedPools").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_tracked_pools(&self, pools: &[Pool]) -> Result<(), TornadoVaultDbError> {
        self.put(b"trackedPools", &postcard::to_stdvec(pools)?)
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait PoolDb: Database {
    /// Returns the exclusive end of the synced block range, if the pool has been synced.
    async fn latest_block(&self) -> Result<Option<u64>, TornadoVaultDbError> {
        Ok(self.cursor().await?.map(|cursor| cursor.latest_block))
    }

    /// Writes `snapshot` as the next chunk without marking it synced. Until
    /// [`PoolDb::commit_snapshot`] runs, a re-sync overwrites the same chunk.
    async fn put_snapshot(&self, snapshot: &Snapshot) -> Result<(), TornadoVaultDbError> {
        if snapshot.events.is_empty() {
            return Ok(());
        }
        let chunks = self.cursor().await?.unwrap_or_default().chunks;
        self.put(&snapshot_key(chunks), &postcard::to_stdvec(snapshot)?)
            .await?;
        Ok(())
    }

    /// Marks `snapshot` synced by advancing the cursor past it in a single write. Call this
    /// last, so a sync interrupted before it is redone from the same block.
    async fn commit_snapshot(&self, snapshot: &Snapshot) -> Result<(), TornadoVaultDbError> {
        let chunks = self.cursor().await?.unwrap_or_default().chunks;
        let cursor = SyncCursor {
            latest_block: snapshot.range.end,
            chunks: chunks + u64::from(!snapshot.events.is_empty()),
        };
        self.put(b"cursor", &postcard::to_stdvec(&cursor)?).await?;
        Ok(())
    }

    /// Returns every committed snapshot chunk, in the order they were synced.
    async fn snapshots(&self) -> Result<Vec<Snapshot>, TornadoVaultDbError> {
        let chunks = self.cursor().await?.unwrap_or_default().chunks;
        let mut snapshots = Vec::new();
        for index in 0..chunks {
            let Some(bytes) = self.get(&snapshot_key(index)).await? else {
                return Err(TornadoVaultDbError::MissingSnapshot(index));
            };
            snapshots.push(postcard::from_bytes(&bytes)?);
        }
        Ok(snapshots)
    }

    async fn cursor(&self) -> Result<Option<SyncCursor>, TornadoVaultDbError> {
        let Some(bytes) = self.get(b"cursor").await? else {
            return Ok(None);
        };
        Ok(Some(postcard::from_bytes(&bytes)?))
    }

    async fn notes(&self) -> Result<Vec<NoteRecord>, TornadoVaultDbError> {
        let Some(bytes) = self.get(b"notes").await? else {
            return Ok(vec![]);
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    async fn put_notes(&self, notes: &[NoteRecord]) -> Result<(), TornadoVaultDbError> {
        self.put(b"notes", &postcard::to_stdvec(notes)?).await?;
        Ok(())
    }
}

/// The public values of a note this vault derived. The spending secrets are re-derived from the
/// signer when needed and never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRecord {
    /// The keychain nonce the note was derived from.
    pub nonce: u64,
    pub commitment: Field,
    pub nullifier_hash: Field,
    /// `None` until the deposit is seen on-chain.
    pub leaf_index: Option<u32>,
    pub spent: bool,
}

/// How far a pool is synced. Kept in one record so it advances atomically.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SyncCursor {
    /// The exclusive end of the synced block range.
    pub latest_block: u64,
    /// The number of committed snapshot chunks.
    pub chunks: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum TornadoVaultDbError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(#[from] postcard::Error),
    #[error("missing snapshot chunk {0}")]
    MissingSnapshot(u64),
}

#[async_trait::async_trait]
impl<D: Database + ?Sized> TornadoVaultDb for D {}
#[async_trait::async_trait]
impl<D: Database + ?Sized> PoolDb for D {}

fn snapshot_key(index: u64) -> Vec<u8> {
    format!("snapshot:{index}").into_bytes()
}
