use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, B256, U256};
use alloy_signer::{Signer, k256::ecdsa::SigningKey};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, sol};
pub use amount::{
    AmountError, PoolSelection, cover_notes, deposit_pools, deposit_pools_for_asset,
    deposit_pools_for_asset_denominations, deposit_pools_for_asset_with_selection,
    deposit_pools_for_denominations, deposit_pools_with_selection, exact_notes,
    exact_notes_for_asset, format_amount, format_eth, parse_amount, parse_eth, parse_wei,
    resolve_asset, withdrawal_notes,
};
pub use kohaku_tornadocash::{Asset, Pool};
use kohaku_tornadocash::{
    Deposit, Note, Withdrawal, WithdrawalError,
    event::SyncEvent,
    merkle_tree::{MerkleProof, MerkleTree, MerkleTreeExt},
};
use kohaku_tornadocash_keychain::{Keychain, KeychainError, recover};
use kohaku_tornadocash_syncer::{DynSyncer, Syncer};
use tokio::sync::Mutex;
pub use userop::{
    IncludedWithdrawal, PaymasterWithdrawal, PaymasterWithdrawalRequest, QuotedWithdrawal,
    UserOpError, WithdrawalCost, quote_paymaster_withdrawal,
};

use crate::{
    asset::AssetId,
    call::Call,
    database::{Database, scoped::ScopedDatabase},
    network::NetworkEndpoint,
    vault::{
        Balance, Vault, VaultError, VaultId, Visibility,
        tornadocash::{
            db::{NoteRecord, PoolDb, TornadoVaultDb},
            syncer::{ChainedSyncer, NetworkEndpointSyncer},
        },
    },
};

mod amount;
mod db;
mod estimation;
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod reservation_tests;
mod syncer;
mod userop;
#[cfg(test)]
mod userop_tests;

sol!(
    contract Erc20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address owner) external view returns (uint256);
    }
);

/// `TornadoVault` stores assets as Tornado Cash notes derived from its signer.
///
/// Each deposit creates one note in a single pool, and each withdrawal spends one note. Choosing
/// which pools to deposit to and which notes to withdraw is left to the caller.
pub struct TornadoVault<S: Signer + Send + Sync> {
    selection: PoolSelection,
    trees_cache: Mutex<HashMap<Pool, MerkleTree>>,
    address: Address,
    keychain: Keychain<S>,
    provider: Arc<dyn NetworkEndpoint>,
    syncer: DynSyncer,
    db: Arc<dyn Database>,
}

/// A note held by the vault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TornadoNote {
    pub pool: Pool,
    pub nonce: u64,
    /// The note's leaf in the pool's merkle tree, or `None` until its deposit is mined.
    pub leaf_index: Option<u32>,
    pub spent: bool,
}

/// Public metadata and ordered calls for a durably reserved deposit. No spending secrets.
#[derive(Clone, Debug)]
pub struct PreparedDeposit {
    pub pool: Pool,
    pub nonce: u64,
    pub commitment: B256,
    pub calls: Vec<Call>,
}

/// One unspent note, in the shape a balance readout can sum and print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnspentNote {
    pub asset: AssetId,
    pub symbol: String,
    pub decimals: u8,
    pub token: Option<Address>,
    pub amount_wei: U256,
    pub pending: bool,
    pub pool_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TornadoVaultError {
    #[error(transparent)]
    Amount(#[from] AmountError),
    #[error("extended pool {0} is disabled")]
    ExtendedDisabled(String),
    #[error("database error: {0}")]
    Database(#[from] db::TornadoVaultDbError),
    #[error("provider error: {0}")]
    Provider(#[from] crate::network::NetworkEndpointError),
    #[error("syncer error: {0}")]
    Syncer(#[from] kohaku_tornadocash_syncer::SyncerError),
    #[error("keychain error: {0}")]
    Keychain(#[from] KeychainError),
    #[error("withdrawal error: {0}")]
    Withdrawal(#[from] WithdrawalError),
    #[error("invalid ERC-20 balance response: {0}")]
    TokenBalance(String),
    #[error("merkle tree error: {0}")]
    MerkleTree(String),
    #[error("Tornado keychain nonce exhausted")]
    NonceExhausted,
    #[error("note {0} has not been mined")]
    NoteNotMined(u64),
    #[error("note {0} has already been spent")]
    NoteSpent(u64),
    #[error("no pools for asset {0:?}")]
    UnsupportedAsset(AssetId),
    #[error("unsupported vault id: {0:?}")]
    UnsupportedVaultId(VaultId),
}

impl TornadoVault<PrivateKeySigner> {
    /// A vault whose notes come from `signing_key`. Pools sync from `indexer_url` when it is set,
    /// then the endpoint.
    ///
    /// # Errors
    /// Returns an error if the Saga client cannot be obtained.
    pub fn open(
        signing_key: SigningKey,
        db: Arc<dyn Database>,
        endpoint: Arc<dyn NetworkEndpoint>,
        indexer_url: Option<&str>,
    ) -> Result<Self, TornadoVaultError> {
        let syncer = ChainedSyncer::tornado(endpoint.clone(), indexer_url)?;
        Ok(Self::with_syncer(
            PrivateKeySigner::from_signing_key(signing_key),
            db,
            endpoint,
            DynSyncer::new(syncer),
        ))
    }
}

impl<S: Signer + Send + Sync> TornadoVault<S> {
    pub async fn deposit_pools(&self, amount: U256) -> Result<Vec<Pool>, TornadoVaultError> {
        Ok(deposit_pools_with_selection(
            amount,
            self.provider.network_id().await?,
            self.selection,
        )?)
    }
    pub async fn deposit_pools_for_asset(
        &self,
        amount: U256,
        asset: &Asset,
    ) -> Result<Vec<Pool>, TornadoVaultError> {
        Ok(deposit_pools_for_asset_with_selection(
            amount,
            self.provider.network_id().await?,
            asset,
            self.selection,
        )?)
    }

    pub async fn deposit_pools_for_denominations(
        &self,
        denominations: &[U256],
    ) -> Result<Vec<Pool>, TornadoVaultError> {
        Ok(deposit_pools_for_denominations(
            denominations,
            self.provider.network_id().await?,
            self.selection,
        )?)
    }

    pub async fn deposit_pools_for_asset_denominations(
        &self,
        denominations: &[U256],
        asset: &Asset,
    ) -> Result<Vec<Pool>, TornadoVaultError> {
        Ok(deposit_pools_for_asset_denominations(
            denominations,
            self.provider.network_id().await?,
            asset,
            self.selection,
        )?)
    }

    async fn operation_lock(&self) -> Result<crate::database::DatabaseGuard, TornadoVaultError> {
        Ok(self
            .db
            .lock()
            .await
            .map_err(db::TornadoVaultDbError::from)?)
    }

    pub fn new(signer: S, db: Arc<dyn Database>, provider: Arc<dyn NetworkEndpoint>) -> Self {
        let syncer = DynSyncer::new(NetworkEndpointSyncer::new(provider.clone()));
        Self::with_syncer(signer, db, provider, syncer)
    }

    /// `syncer` replaces the endpoint log syncer, for a chain that can read a CDN first.
    #[must_use]
    pub fn with_syncer(
        signer: S,
        db: Arc<dyn Database>,
        provider: Arc<dyn NetworkEndpoint>,
        syncer: DynSyncer,
    ) -> Self {
        Self {
            selection: PoolSelection::default(),
            trees_cache: Mutex::new(HashMap::new()),
            address: signer.address(),
            keychain: Keychain::new(signer),
            provider,
            syncer,
            db,
        }
    }

    #[must_use]
    pub fn with_pool_selection(mut self, selection: PoolSelection) -> Self {
        self.selection = selection;
        self
    }

    fn check_pool(&self, pool: &Pool) -> Result<(), TornadoVaultError> {
        if !self.selection.includes(pool) {
            return Err(TornadoVaultError::ExtendedDisabled(pool.id()));
        }
        Ok(())
    }

    /// Syncs the given pools' events up to the latest block, updating the vault's notes.
    pub async fn sync(&self, pools: &[Pool]) -> Result<(), TornadoVaultError> {
        for pool in pools {
            self.check_pool(pool)?;
        }
        let _guard = self.operation_lock().await?;
        self.sync_unlocked(pools).await
    }

    async fn sync_unlocked(&self, pools: &[Pool]) -> Result<(), TornadoVaultError> {
        //? `block_height` is inclusive, sync ranges are half-open.
        let end = self.provider.block_height().await?.saturating_add(1);

        for pool in pools {
            self.sync_pool(pool, end).await?;
        }

        Ok(())
    }

    /// Recovers all notes derived from the vault's signer for the given pools.
    ///
    /// Keeps local reservations, including unbroadcast deposits. Recovery never rewinds the
    /// nonce counter. `gap_limit` recovery defaults apply only when discovering unknown notes.
    pub async fn recover(&self, pools: &[Pool]) -> Result<(), TornadoVaultError> {
        self.recover_with_gap(pools, None).await
    }

    /// Recovers with an explicit discovery gap for seed-only restores after abandoned preparations.
    pub async fn recover_with_gap(
        &self,
        pools: &[Pool],
        gap_limit: Option<u64>,
    ) -> Result<(), TornadoVaultError> {
        self.recover_with_gap_and_progress(pools, gap_limit, |_, _, _| {})
            .await
    }

    async fn recover_with_gap_and_progress(
        &self,
        pools: &[Pool],
        gap_limit: Option<u64>,
        mut progress: impl FnMut(usize, usize, &Pool),
    ) -> Result<(), TornadoVaultError> {
        for pool in pools {
            self.check_pool(pool)?;
        }
        let _guard = self.operation_lock().await?;
        self.track(pools).await?;
        let end = self.provider.block_height().await?.saturating_add(1);
        for (index, pool) in pools.iter().enumerate() {
            progress(index + 1, pools.len(), pool);
            self.sync_pool(pool, end).await?;
        }

        let mut snapshots = Vec::new();
        for pool in pools {
            snapshots.extend(self.pool_db(pool).snapshots().await?);
        }
        let recovered = recover(&self.keychain, &snapshots, gap_limit).await?;
        // Publish the recovered high-water mark before writing recovered records, so an
        // interrupted rescan cannot leave newly discovered notes ahead of the counter.
        let recovered_next = recovered
            .iter()
            .map(|found| {
                found
                    .nonce
                    .checked_add(1)
                    .ok_or(TornadoVaultError::NonceExhausted)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .unwrap_or(0);
        let stored_next = self.db.next_nonce().await?;
        self.db
            .put_next_nonce(recovered_next.max(stored_next))
            .await?;

        for pool in pools {
            let mut notes = self.pool_db(pool).notes().await?;
            for found in recovered.iter().filter(|found| found.pool == *pool) {
                let note = self.derive_note(found.nonce, pool).await?;
                let record = NoteRecord {
                    nonce: found.nonce,
                    leaf_index: Some(found.deposit.leaf_index),
                    spent: found.withdrawal.is_some(),
                    commitment: note.commitment(),
                    nullifier_hash: note.nullifier_hash(),
                };
                if let Some(existing) = notes
                    .iter_mut()
                    .find(|existing| existing.commitment == record.commitment)
                {
                    *existing = record;
                } else {
                    notes.push(record);
                }
            }
            self.pool_db(pool).put_notes(&notes).await?;
        }

        Ok(())
    }

    /// Recovers notes in every known pool on the vault's current chain.
    ///
    /// Nonces are shared across pools. Scanning stops after 20 consecutive nonces with no
    /// deposit in any pool. Returns how many unspent notes were stored.
    pub async fn recover_chain(&self) -> Result<usize, TornadoVaultError> {
        self.recover_chain_with_gap(None).await
    }

    /// Discovers chain notes with an optional consecutive-unused-nonce limit.
    pub async fn recover_chain_with_gap(
        &self,
        gap_limit: Option<u64>,
    ) -> Result<usize, TornadoVaultError> {
        self.recover_chain_with_gap_and_progress(gap_limit, |_, _, _| {})
            .await
    }

    pub async fn recover_chain_with_gap_and_progress(
        &self,
        gap_limit: Option<u64>,
        progress: impl FnMut(usize, usize, &Pool),
    ) -> Result<usize, TornadoVaultError> {
        let chain_id = self.provider.network_id().await?;
        let pools: Vec<Pool> = Pool::POOLS
            .iter()
            .filter(|pool| pool.chain_id == chain_id && self.selection.includes(pool))
            .cloned()
            .collect();
        self.recover_with_gap_and_progress(&pools, gap_limit, progress)
            .await?;
        let notes = self.pool_notes(&pools).await?;
        Ok(notes
            .iter()
            .filter(|note| !note.spent && note.leaf_index.is_some())
            .count())
    }

    /// Returns the calls that deposit one new note into `pool`.
    ///
    /// ERC-20 deposits are preceded by an approval for the pool's denomination. The pool is
    /// tracked from then on.
    pub async fn deposit(&self, pool: &Pool) -> Result<Vec<Call>, TornadoVaultError> {
        let deposits = self.prepare_deposits(std::slice::from_ref(pool)).await?;
        Ok(deposits
            .into_iter()
            .flat_map(|deposit| deposit.calls)
            .collect())
    }

    /// Reserves nonces, records pending notes, and returns calls without checking balances or
    /// broadcasting. Nonces are persisted before derivation and are never rolled back on error.
    /// A partial failure consumes reservations but exposes no calldata; gaps are safe.
    pub async fn prepare_deposits(
        &self,
        pools: &[Pool],
    ) -> Result<Vec<PreparedDeposit>, TornadoVaultError> {
        for pool in pools {
            self.check_pool(pool)?;
        }
        let _guard = self.operation_lock().await?;
        self.track(pools).await?;
        let mut deposits = Vec::with_capacity(pools.len());
        for pool in pools {
            deposits.push(self.deposit_note(pool).await?);
        }
        Ok(deposits)
    }

    /// Returns a list of every note derived from the vault's signer.
    ///
    /// Syncs the tracked pools on the current network. Returns pending, mined,
    /// and spent notes.
    pub async fn notes(&self) -> Result<Vec<TornadoNote>, TornadoVaultError> {
        let _guard = self.operation_lock().await?;
        let chain_id = self.provider.network_id().await?;
        let pools: Vec<Pool> = self
            .db
            .tracked_pools()
            .await?
            .into_iter()
            .filter(|pool| pool.chain_id == chain_id && self.selection.includes(pool))
            .collect();

        self.sync_unlocked(&pools).await?;
        self.pool_notes(&pools).await
    }

    /// Forgets unmined deposits after syncing tracked pools. Nonces are never reused.
    pub async fn clear_pending(
        &self,
        nonce: Option<u64>,
    ) -> Result<Vec<TornadoNote>, TornadoVaultError> {
        let _guard = self.operation_lock().await?;
        let chain_id = self.provider.network_id().await?;
        let pools: Vec<Pool> = self
            .db
            .tracked_pools()
            .await?
            .into_iter()
            .filter(|pool| pool.chain_id == chain_id)
            .collect();
        let end = self.provider.block_height().await?.saturating_add(1);
        for pool in &pools {
            self.sync_pool(pool, end).await?;
        }
        let mut cleared = Vec::new();
        for pool in &pools {
            let db = self.pool_db(pool);
            let mut records = db.notes().await?;
            records.retain(|record| {
                if record.leaf_index.is_some()
                    || record.spent
                    || nonce.is_some_and(|n| n != record.nonce)
                {
                    return true;
                }
                cleared.push(TornadoNote {
                    pool: pool.clone(),
                    nonce: record.nonce,
                    leaf_index: None,
                    spent: false,
                });
                false
            });
            db.put_notes(&records).await?;
        }
        Ok(cleared)
    }

    /// Returns the call that withdraws `note` to `to`.
    ///
    /// Withdrawing several notes to the same recipient links them on-chain.
    pub async fn withdraw(
        &self,
        note: &TornadoNote,
        to: &VaultId,
    ) -> Result<Call, TornadoVaultError> {
        #[expect(irrefutable_let_patterns)]
        let VaultId::Address(recipient) = to else {
            return Err(TornadoVaultError::UnsupportedVaultId(to.clone()));
        };
        let (withdrawal, proof) = self.withdrawal_inputs(note, *recipient).await?;
        let proven = withdrawal.prove(&proof, &mut rand::rng())?;
        let call = proven.call();
        Ok(Call::new(call.target, call.input, call.value))
    }

    /// The unproven withdrawal of `note` to `recipient`, and the merkle proof it commits to.
    ///
    /// Syncs the pool first, so the proof is against a root still in the pool's on-chain history.
    /// The caller proves it, which is what a paymaster user operation does for its sponsoring note.
    pub async fn withdrawal_inputs(
        &self,
        note: &TornadoNote,
        recipient: Address,
    ) -> Result<(Withdrawal, MerkleProof), TornadoVaultError> {
        self.check_pool(&note.pool)?;
        if note.spent {
            return Err(TornadoVaultError::NoteSpent(note.nonce));
        }
        let Some(leaf_index) = note.leaf_index else {
            return Err(TornadoVaultError::NoteNotMined(note.nonce));
        };

        self.sync(std::slice::from_ref(&note.pool)).await?;
        let derived = self.derive_note(note.nonce, &note.pool).await?;
        let proof = self.merkle_proof(&note.pool, leaf_index).await?;
        Ok((Withdrawal::new(&note.pool, derived, recipient), proof))
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

    fn visibility(&self) -> Visibility {
        Visibility::Private
    }

    async fn balance(&self, asset: &AssetId) -> Result<Balance, VaultError> {
        let _guard = self.operation_lock().await?;
        let pools = self.tracked_pools(asset).await?;
        self.sync_unlocked(&pools).await?;

        let notes = self.pool_notes(&pools).await?;
        Ok(note_balance(&notes))
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

        // Another process may have advanced snapshots since this vault last built its tree.
        self.trees_cache.lock().await.remove(pool);

        let mut notes = pool_db.notes().await?;
        apply_events(&mut notes, snapshot.events());
        pool_db.put_notes(&notes).await?;

        //? Last, so a sync interrupted before here is redone from the same block. Every step
        //? above is safe to repeat.
        pool_db.commit_snapshot(&snapshot).await?;
        Ok(())
    }

    async fn pool_notes(&self, pools: &[Pool]) -> Result<Vec<TornadoNote>, TornadoVaultError> {
        let mut notes = Vec::new();
        for pool in pools {
            let records = self.pool_db(pool).notes().await?;
            notes.extend(records.into_iter().map(|record| TornadoNote {
                pool: pool.clone(),
                nonce: record.nonce,
                leaf_index: record.leaf_index,
                spent: record.spent,
            }));
        }
        Ok(notes)
    }

    /// Reserves the next nonce for a new note in `pool` and returns the call that deposits it.
    async fn deposit_note(&self, pool: &Pool) -> Result<PreparedDeposit, TornadoVaultError> {
        //? Reserve the nonce before deriving the note to avoid re-using a nonce.
        let nonce = self.db.next_nonce().await?;
        self.db
            .put_next_nonce(
                nonce
                    .checked_add(1)
                    .ok_or(TornadoVaultError::NonceExhausted)?,
            )
            .await?;
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

        let commitment = note.commitment().into();
        let deposit = Deposit::new(pool, note);
        let mut calls: Vec<Call> = approval(pool).into_iter().collect();
        let call = deposit.call();
        calls.push(Call::new(call.target, call.input, call.value));
        Ok(PreparedDeposit {
            pool: pool.clone(),
            nonce,
            commitment,
            calls,
        })
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
            tree.splice_events(&snapshot)
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

/// The approval that lets `pool` pull one deposit, or `None` for a native pool.
fn approval(pool: &Pool) -> Option<Call> {
    let Asset::Erc20 { address, .. } = &pool.asset else {
        return None;
    };

    let amount = U256::from(pool.amount_wei);
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

/// Mined unspent notes are spendable. Unmined unspent notes are pending. Spent notes add nothing.
fn note_balance(notes: &[TornadoNote]) -> Balance {
    let mut balance = Balance::default();
    for note in notes.iter().filter(|note| !note.spent) {
        let amount = U256::from(note.pool.amount_wei);
        if note.leaf_index.is_some() {
            balance.spendable = balance.spendable.saturating_add(amount);
        } else {
            balance.pending = balance.pending.saturating_add(amount);
        }
    }
    balance
}

/// The unspent notes, mined and still waiting to be mined.
#[must_use]
pub fn unspent_holdings(notes: &[TornadoNote]) -> Vec<UnspentNote> {
    notes
        .iter()
        .filter(|note| !note.spent)
        .map(|note| {
            let (asset, token) = match &note.pool.asset {
                Asset::Native { .. } => (AssetId::Native, None),
                Asset::Erc20 { address, .. } => (AssetId::Erc20(*address), Some(*address)),
            };
            UnspentNote {
                asset,
                symbol: note.pool.symbol().to_string(),
                decimals: note.pool.asset.decimals(),
                token,
                amount_wei: U256::from(note.pool.amount_wei),
                pending: note.leaf_index.is_none(),
                pool_id: note.pool.id(),
            }
        })
        .collect()
}

/// Reads an ERC-20 balance through the configured endpoint.
pub async fn token_balance(
    endpoint: &dyn NetworkEndpoint,
    token: Address,
    owner: Address,
) -> Result<U256, TornadoVaultError> {
    let data = Erc20::balanceOfCall::new((owner,)).abi_encode();
    let result = endpoint
        .call(
            alloy_rpc_types_eth::TransactionRequest::default()
                .to(token)
                .input(data.into()),
        )
        .await?;
    Erc20::balanceOfCall::abi_decode_returns(&result)
        .map_err(|error| TornadoVaultError::TokenBalance(error.to_string()))
}

/// ERC-20 contract address, or `None` for a native asset.
#[must_use]
pub fn token_address(asset: &Asset) -> Option<Address> {
    match asset {
        Asset::Erc20 { address, .. } => Some(*address),
        Asset::Native { .. } => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use alloy_primitives::address;

    use super::*;

    fn note(pool: Pool, leaf_index: Option<u32>, spent: bool) -> TornadoNote {
        TornadoNote {
            pool,
            nonce: 0,
            leaf_index,
            spent,
        }
    }

    #[test]
    fn unmined_unspent_notes_are_pending_and_spent_notes_are_ignored() {
        let notes = [
            note(Pool::ETHEREUM_ETHER_01, Some(1), false),
            note(Pool::ETHEREUM_ETHER_1, None, false),
            note(Pool::ETHEREUM_ETHER_10, Some(2), true),
        ];
        let balance = note_balance(&notes);
        assert_eq!(
            balance.spendable,
            U256::from(Pool::ETHEREUM_ETHER_01.amount_wei)
        );
        assert_eq!(
            balance.pending,
            U256::from(Pool::ETHEREUM_ETHER_1.amount_wei)
        );

        let holdings = unspent_holdings(&notes);
        assert_eq!(holdings.len(), 2);
        assert_eq!(holdings[0].asset, AssetId::Native);
        assert!(!holdings[0].pending);
        assert_eq!(holdings[0].pool_id, Pool::ETHEREUM_ETHER_01.id());
        assert!(holdings[1].pending);

        let dai_address = address!("0x6B175474E89094C44Da98b954EedeAC495271d0F");
        let dai = unspent_holdings(&[note(Pool::ETHEREUM_DAI_100, Some(0), false)]);
        assert_eq!(dai[0].asset, AssetId::Erc20(dai_address));
        assert_eq!(dai[0].symbol, "dai");
        assert_eq!(dai[0].token, Some(dai_address));
    }
}
