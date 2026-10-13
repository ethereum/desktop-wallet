use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, U256};
use alloy_signer::Signer;
use alloy_sol_types::{SolCall, sol};
use kohaku_tornadocash::{
    Asset, Deposit, Note, Pool, Withdrawal, WithdrawalError,
    merkle_tree::{MerkleProof, MerkleTree, MerkleTreeExt},
    syncer::{DynSyncer, SyncEvent, Syncer},
};
use kohaku_tornadocash_keychain::{Keychain, KeychainError, recover};
use tokio::sync::Mutex;

use crate::{
    asset::AssetId,
    call::Call,
    database::{Database, scoped::ScopedDatabase},
    network::NetworkEndpoint,
    vault::{
        Vault, VaultError, VaultId,
        tornadocash::{
            db::{NoteRecord, PoolDb, TornadoVaultDb},
            syncer::NetworkEndpointSyncer,
        },
    },
};

mod db;
mod syncer;

sol!(
    contract Erc20 {
        function approve(address spender, uint256 amount) external returns (bool);
    }
);

/// `TornadoVault` stores assets as Tornado Cash notes derived from its signer.
///
/// Deposits and withdrawals take an amount and split it into one note per pool denomination,
/// so amounts must be expressible in the pools' denominations.
pub struct TornadoVault<S: Signer + Send + Sync> {
    trees_cache: Mutex<HashMap<Pool, MerkleTree>>,
    address: Address,
    keychain: Keychain<S>,
    provider: Arc<dyn NetworkEndpoint>,
    syncer: DynSyncer,
    db: Arc<dyn Database>,
}

/// A mined, unspent note.
struct UnspentNote {
    pool: Pool,
    nonce: u64,
    leaf_index: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum TornadoVaultError {
    #[error("database error: {0}")]
    Database(#[from] db::TornadoVaultDbError),
    #[error("provider error: {0}")]
    Provider(#[from] crate::network::NetworkEndpointError),
    #[error("syncer error: {0}")]
    Syncer(#[from] kohaku_tornadocash::syncer::SyncerError),
    #[error("keychain error: {0}")]
    Keychain(#[from] KeychainError),
    #[error("withdrawal error: {0}")]
    Withdrawal(#[from] WithdrawalError),
    #[error("merkle tree error: {0}")]
    MerkleTree(String),
    #[error("amount cannot be made from the pools' denominations")]
    UnrepresentableAmount,
    #[error("no pools for asset {0:?}")]
    UnsupportedAsset(AssetId),
    #[error("unsupported vault id: {0:?}")]
    UnsupportedVaultId(VaultId),
}

impl<S: Signer + Send + Sync> TornadoVault<S> {
    pub fn new(signer: S, db: Arc<dyn Database>, provider: Arc<dyn NetworkEndpoint>) -> Self {
        let syncer = DynSyncer::new(NetworkEndpointSyncer::new(provider.clone()));

        Self {
            trees_cache: Mutex::new(HashMap::new()),
            address: signer.address(),
            keychain: Keychain::new(signer),
            provider,
            syncer,
            db,
        }
    }

    /// Syncs the given pools' events up to the latest block, updating the vault's notes.
    pub async fn sync(&self, pools: &[Pool]) -> Result<(), TornadoVaultError> {
        //? `block_height` is inclusive, sync ranges are half-open.
        let end = self.provider.block_height().await?.saturating_add(1);

        for pool in pools {
            self.sync_pool(pool, end).await?;
        }

        Ok(())
    }

    /// Recovers all notes derived from the vault's signer for the given pools.
    ///
    /// Replaces the pools' stored notes, so deposits not yet mined are forgotten. A later
    /// `recover` finds them once they are.
    pub async fn recover(&self, pools: &[Pool]) -> Result<(), TornadoVaultError> {
        self.track(pools).await?;
        self.sync(pools).await?;

        let mut snapshots = Vec::new();
        for pool in pools {
            snapshots.extend(self.pool_db(pool).snapshots().await?);
        }
        let recovered = recover(&self.keychain, &snapshots, None).await?;

        for pool in pools {
            let mut notes = Vec::new();
            for found in recovered.iter().filter(|found| found.pool == *pool) {
                let note = self.derive_note(found.nonce, pool).await?;
                notes.push(NoteRecord {
                    nonce: found.nonce,
                    leaf_index: Some(found.deposit.leaf_index),
                    spent: found.withdrawal.is_some(),
                    commitment: note.commitment(),
                    nullifier_hash: note.nullifier_hash(),
                });
            }
            self.pool_db(pool).put_notes(&notes).await?;
        }

        let recovered_next = recovered.iter().map(|found| found.nonce + 1).max();
        let stored_next = self.db.next_nonce().await?;
        self.db
            .put_next_nonce(recovered_next.unwrap_or(0).max(stored_next))
            .await?;
        Ok(())
    }

    /// Returns the calls that deposit `amount` of `asset`, one note per pool denomination.
    ///
    /// ERC-20 deposits are preceded by one approval per pool, covering all of that pool's notes.
    /// The pools used are tracked from then on.
    pub async fn deposit(
        &self,
        asset: &AssetId,
        amount: U256,
    ) -> Result<Vec<Call>, TornadoVaultError> {
        let pools = self.pools(asset).await?;
        let notes = split(amount, &pools)?;
        let per_pool: Vec<&[Pool]> = notes.chunk_by(PartialEq::eq).collect();

        let used: Vec<Pool> = per_pool.iter().map(|same| same[0].clone()).collect();
        self.track(&used).await?;

        let mut calls: Vec<Call> = per_pool
            .iter()
            .filter_map(|same| approval(&same[0], same.len()))
            .collect();
        for pool in &notes {
            calls.push(self.deposit_note(pool).await?);
        }
        Ok(calls)
    }

    /// Returns the calls that withdraw `amount` of `asset` to `to`, one call per spent note.
    ///
    /// Withdrawing several notes to the same recipient links them on-chain.
    pub async fn withdraw(
        &self,
        to: &VaultId,
        asset: &AssetId,
        amount: U256,
    ) -> Result<Vec<Call>, TornadoVaultError> {
        #[expect(irrefutable_let_patterns)]
        let VaultId::Address(recipient) = to else {
            return Err(TornadoVaultError::UnsupportedVaultId(to.clone()));
        };

        let pools = self.tracked_pools(asset).await?;
        //? Proofs must be against a root still in the pool's on-chain root history.
        self.sync(&pools).await?;
        let unspent = self.unspent_notes(&pools).await?;

        let mut calls = Vec::new();
        for note in select(amount, unspent)? {
            calls.push(self.withdraw_note(&note, *recipient).await?);
        }
        Ok(calls)
    }
}

#[async_trait::async_trait]
impl<S: Signer + Send + Sync> Vault for TornadoVault<S> {
    fn tag(&self) -> &'static str {
        "tornado"
    }

    fn id(&self) -> VaultId {
        VaultId::Address(self.address)
    }

    async fn balance(&self, asset: &AssetId) -> Result<U256, VaultError> {
        let pools = self.tracked_pools(asset).await?;
        self.sync(&pools).await?;

        let unspent = self.unspent_notes(&pools).await?;
        Ok(unspent
            .iter()
            .map(|note| U256::from(note.pool.amount_wei))
            .sum())
    }
}

impl<S: Signer + Send + Sync> TornadoVault<S> {
    /// The current network's pools for `asset`, largest denomination first.
    async fn pools(&self, asset: &AssetId) -> Result<Vec<Pool>, TornadoVaultError> {
        let chain_id = self.provider.network_id().await?;
        let mut pools: Vec<Pool> = Pool::POOLS
            .iter()
            .filter(|pool| pool.chain_id == chain_id && is_eq(pool, asset))
            .cloned()
            .collect();

        if pools.is_empty() {
            return Err(TornadoVaultError::UnsupportedAsset(asset.clone()));
        }
        pools.sort_by_key(|pool| std::cmp::Reverse(pool.amount_wei));
        Ok(pools)
    }

    /// The tracked pools for `asset`, largest denomination first.
    async fn tracked_pools(&self, asset: &AssetId) -> Result<Vec<Pool>, TornadoVaultError> {
        let tracked = self.db.tracked_pools().await?;
        let pools = self.pools(asset).await?;
        Ok(pools
            .into_iter()
            .filter(|pool| tracked.contains(pool))
            .collect())
    }

    /// Adds `pools` to the tracked pools.
    async fn track(&self, pools: &[Pool]) -> Result<(), TornadoVaultError> {
        let mut tracked = self.db.tracked_pools().await?;
        let untracked: Vec<Pool> = pools
            .iter()
            .filter(|pool| !tracked.contains(pool))
            .cloned()
            .collect();
        if untracked.is_empty() {
            return Ok(());
        }

        tracked.extend(untracked);
        self.db.put_tracked_pools(&tracked).await?;
        Ok(())
    }

    async fn sync_pool(&self, pool: &Pool, end: u64) -> Result<(), TornadoVaultError> {
        let pool_db = self.pool_db(pool);
        let start = pool_db.latest_block().await?.unwrap_or(pool.deployed_block);

        let snapshot = self.syncer.sync_range(pool, start, end).await?;
        pool_db.put_snapshot(&snapshot).await?;

        if let Some(tree) = self.trees_cache.lock().await.get_mut(pool) {
            tree.splice_events(&snapshot.events)
                .map_err(|e| TornadoVaultError::MerkleTree(e.to_string()))?;
        }

        let mut notes = pool_db.notes().await?;
        apply_events(&mut notes, &snapshot.events);
        pool_db.put_notes(&notes).await?;

        //? Last, so a sync interrupted before here is redone from the same block. Every step
        //? above is safe to repeat.
        pool_db.commit_snapshot(&snapshot).await?;
        Ok(())
    }

    async fn unspent_notes(&self, pools: &[Pool]) -> Result<Vec<UnspentNote>, TornadoVaultError> {
        let mut unspent = Vec::new();
        for pool in pools {
            let notes = self.pool_db(pool).notes().await?;
            unspent.extend(
                notes
                    .into_iter()
                    .filter(|note| !note.spent)
                    .filter_map(|note| {
                        Some(UnspentNote {
                            pool: pool.clone(),
                            nonce: note.nonce,
                            leaf_index: note.leaf_index?,
                        })
                    }),
            );
        }
        Ok(unspent)
    }

    /// Reserves the next nonce for a new note in `pool` and returns the call that deposits it.
    async fn deposit_note(&self, pool: &Pool) -> Result<Call, TornadoVaultError> {
        //? Reserve the nonce before deriving the note to avoid re-using a nonce.
        let nonce = self.db.next_nonce().await?;
        self.db.put_next_nonce(nonce + 1).await?;
        let note = self.derive_note(nonce, pool).await?;

        let pool_db = self.pool_db(pool);
        let mut notes = pool_db.notes().await?;
        notes.push(NoteRecord {
            nonce,
            commitment: note.commitment(),
            nullifier_hash: note.nullifier_hash(),
            leaf_index: None,
            spent: false,
        });
        pool_db.put_notes(&notes).await?;

        let deposit = Deposit::new(pool, note);
        Ok(Call::new(
            deposit.target(),
            deposit.input().into(),
            deposit.value(),
        ))
    }

    async fn withdraw_note(
        &self,
        unspent: &UnspentNote,
        recipient: Address,
    ) -> Result<Call, TornadoVaultError> {
        let note = self.derive_note(unspent.nonce, &unspent.pool).await?;
        let proof = self.merkle_proof(&unspent.pool, unspent.leaf_index).await?;

        let proven =
            Withdrawal::new(&unspent.pool, note, recipient).prove(&proof, &mut rand::rng())?;
        Ok(Call::new(
            proven.target(),
            proven.input().into(),
            U256::ZERO,
        ))
    }

    async fn merkle_proof(
        &self,
        pool: &Pool,
        leaf_index: u32,
    ) -> Result<MerkleProof, TornadoVaultError> {
        let mut trees = self.trees_cache.lock().await;
        if !trees.contains_key(pool) {
            let tree = self.build_tree(pool).await?;
            trees.insert(pool.clone(), tree);
        }

        trees[pool]
            .proof(leaf_index as usize)
            .map_err(|e| TornadoVaultError::MerkleTree(e.to_string()))
    }

    async fn build_tree(&self, pool: &Pool) -> Result<MerkleTree, TornadoVaultError> {
        let mut tree = MerkleTree::new();
        for snapshot in self.pool_db(pool).snapshots().await? {
            tree.splice_events(&snapshot.events)
                .map_err(|e| TornadoVaultError::MerkleTree(e.to_string()))?;
        }
        Ok(tree)
    }

    async fn derive_note(&self, nonce: u64, pool: &Pool) -> Result<Note, TornadoVaultError> {
        Ok(self.keychain.note(nonce, pool).await?.note)
    }

    fn pool_db(&self, pool: &Pool) -> ScopedDatabase {
        ScopedDatabase::new(self.db.clone(), pool.id().as_bytes())
    }
}

impl From<TornadoVaultError> for VaultError {
    fn from(err: TornadoVaultError) -> Self {
        VaultError::Other(Box::new(err))
    }
}

/// Splits `amount` into pool denominations using the largest denominations first.
///
/// `pools` must be sorted by largest denomination first. Returns an error if
/// `amount` cannot be made from the pools' denominations.
fn split(amount: U256, pools: &[Pool]) -> Result<Vec<Pool>, TornadoVaultError> {
    let mut remaining = amount;
    let mut notes = Vec::new();
    for pool in pools {
        let denomination = U256::from(pool.amount_wei);
        let count = usize::try_from(remaining / denomination)
            .map_err(|_| TornadoVaultError::UnrepresentableAmount)?;
        remaining %= denomination;
        notes.extend(std::iter::repeat_n(pool.clone(), count));
    }

    if !remaining.is_zero() {
        return Err(TornadoVaultError::UnrepresentableAmount);
    }
    Ok(notes)
}

/// Selects notes summing to exactly `amount` using the largest denominations first.
///
/// `unspent` must be sorted by largest denomination first. Returns an error if
/// `amount` cannot be made from the notes.
fn select(amount: U256, unspent: Vec<UnspentNote>) -> Result<Vec<UnspentNote>, TornadoVaultError> {
    let mut remaining = amount;
    let mut selected = Vec::new();
    for note in unspent {
        let denomination = U256::from(note.pool.amount_wei);
        if denomination > remaining {
            continue;
        }
        remaining -= denomination;
        selected.push(note);
    }

    if !remaining.is_zero() {
        return Err(TornadoVaultError::UnrepresentableAmount);
    }
    Ok(selected)
}

/// Marks notes deposited or spent by `events`.
fn apply_events(notes: &mut [NoteRecord], events: &[SyncEvent]) {
    for event in events {
        match event {
            SyncEvent::Deposit(deposit) => notes
                .iter_mut()
                .filter(|note| note.commitment == deposit.commitment)
                .for_each(|note| note.leaf_index = Some(deposit.leaf_index)),
            SyncEvent::Withdrawal(withdrawal) => notes
                .iter_mut()
                .filter(|note| note.nullifier_hash == withdrawal.nullifier_hash)
                .for_each(|note| note.spent = true),
        }
    }
}

/// The approval that lets `pool` pull `notes` deposits, or `None` for a native pool.
fn approval(pool: &Pool, notes: usize) -> Option<Call> {
    let Asset::Erc20 { address, .. } = &pool.asset else {
        return None;
    };

    let amount = U256::from(pool.amount_wei) * U256::from(notes);
    let data = Erc20::approveCall::new((pool.address, amount))
        .abi_encode()
        .into();
    Some(Call::new(*address, data, U256::ZERO))
}

fn is_eq(pool: &Pool, asset: &AssetId) -> bool {
    match (&pool.asset, asset) {
        (Asset::Native { .. }, AssetId::Native) => true,
        (
            Asset::Erc20 {
                address: pool_address,
                ..
            },
            AssetId::Erc20(asset_address),
        ) => pool_address == asset_address,
        _ => false,
    }
}
