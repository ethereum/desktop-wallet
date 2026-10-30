use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, U256};
use alloy_signer::{Signer, k256::ecdsa::SigningKey};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, sol};
pub use amount::{
    AmountError, PoolSelection, cover_notes, deposit_pools, deposit_pools_for_denominations,
    deposit_pools_with_selection, exact_notes, format_eth, parse_eth, parse_wei,
};
use kohaku_tornadocash::{
    Asset, Deposit, Note, Pool, Withdrawal, WithdrawalError,
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
mod syncer;
mod userop;

sol!(
    contract Erc20 {
        function approve(address spender, uint256 amount) external returns (bool);
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
    #[error("merkle tree error: {0}")]
    MerkleTree(String),
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

    /// Selects supported pool families for syncing, recovery and spending.
    #[must_use]
    pub fn with_pool_selection(mut self, selection: PoolSelection) -> Self {
        self.selection = selection;
        self
    }

    /// Plans native ETH deposits using this vault's configured pool set.
    pub async fn deposit_pools(&self, amount: U256) -> Result<Vec<Pool>, TornadoVaultError> {
        Ok(deposit_pools_with_selection(
            amount,
            self.provider.network_id().await?,
            self.selection,
        )?)
    }

    /// Plans exactly the requested native ETH denominations in their supplied order.
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

    /// Selects the fewest eligible held notes summing to `amount`.
    pub async fn exact_notes(&self, amount: U256) -> Result<Vec<TornadoNote>, TornadoVaultError> {
        Ok(exact_notes(amount, &self.notes().await?)?)
    }

    /// Selects the fewest eligible held notes covering `amount`.
    pub async fn cover_notes(&self, amount: U256) -> Result<Vec<TornadoNote>, TornadoVaultError> {
        Ok(cover_notes(amount, &self.notes().await?)?)
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
        self.recover_with_progress(pools, |_, _, _| {}).await
    }

    async fn recover_with_progress(
        &self,
        pools: &[Pool],
        mut progress: impl FnMut(usize, usize, &Pool),
    ) -> Result<(), TornadoVaultError> {
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

    /// Recovers notes in every known pool on the vault's current chain.
    ///
    /// Nonces are shared across pools. Scanning stops after 20 consecutive nonces with no
    /// deposit in any pool. Returns how many unspent notes were stored.
    pub async fn recover_chain(&self) -> Result<usize, TornadoVaultError> {
        self.recover_chain_with_progress(|_, _, _| {}).await
    }

    /// Recovers the chain's pools, reporting each pool before it is synced.
    pub async fn recover_chain_with_progress(
        &self,
        progress: impl FnMut(usize, usize, &Pool),
    ) -> Result<usize, TornadoVaultError> {
        let chain_id = self.provider.network_id().await?;
        let pools: Vec<Pool> = Pool::POOLS
            .iter()
            .filter(|pool| pool.chain_id == chain_id && self.selection.includes(pool))
            .cloned()
            .collect();
        self.recover_with_progress(&pools, progress).await?;
        let notes = self.pool_notes(&pools).await?;
        Ok(notes.iter().filter(|note| !note.spent).count())
    }

    /// Returns the calls that deposit one new note into `pool`.
    ///
    /// ERC-20 deposits are preceded by an approval for the pool's denomination. The pool is
    /// tracked from then on.
    pub async fn deposit(&self, pool: &Pool) -> Result<Vec<Call>, TornadoVaultError> {
        self.check_pool(pool)?;
        self.track(std::slice::from_ref(pool)).await?;

        let mut calls: Vec<Call> = approval(pool).into_iter().collect();
        calls.push(self.deposit_note(pool).await?);
        Ok(calls)
    }

    /// Returns a list of every note derived from the vault's signer.
    ///
    /// Syncs the tracked pools on the current network. Returns pending, mined,
    /// and spent notes.
    pub async fn notes(&self) -> Result<Vec<TornadoNote>, TornadoVaultError> {
        let chain_id = self.provider.network_id().await?;
        let pools: Vec<Pool> = self
            .db
            .tracked_pools()
            .await?
            .into_iter()
            .filter(|pool| pool.chain_id == chain_id && self.selection.includes(pool))
            .collect();

        self.sync(&pools).await?;
        self.pool_notes(&pools).await
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
        let pools = self.tracked_pools(asset).await?;
        self.sync(&pools).await?;

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
            .filter(|pool| {
                pool.chain_id == chain_id && self.selection.includes(pool) && is_eq(pool, asset)
            })
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
        for pool in pools {
            self.check_pool(pool)?;
        }
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
            tree.splice_events(&snapshot)
                .map_err(|e| TornadoVaultError::MerkleTree(e.to_string()))?;
        }

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

        let call = Deposit::new(pool, note).call();
        Ok(Call::new(call.target, call.input, call.value))
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

    fn offline_vault(selection: PoolSelection) -> TornadoVault<PrivateKeySigner> {
        use alloy_provider::{Provider, ProviderBuilder};
        let provider = ProviderBuilder::new()
            .connect_http("http://127.0.0.1:1".parse().expect("URL"))
            .erased();
        TornadoVault::new(
            PrivateKeySigner::random(),
            Arc::new(crate::database::memory::MemoryDatabase::default()),
            Arc::new(crate::network::SimpleNetworkEndpoint::new(provider)),
        )
        .with_pool_selection(selection)
    }

    #[tokio::test]
    async fn disabled_extended_operations_fail_before_rpc_or_storage_changes() {
        let vault = offline_vault(PoolSelection::Classic);
        let pool = Pool::ETHEREUM_ETHER_001;
        assert!(matches!(
            vault.deposit(&pool).await,
            Err(TornadoVaultError::ExtendedDisabled(_))
        ));
        assert!(matches!(
            vault.sync(std::slice::from_ref(&pool)).await,
            Err(TornadoVaultError::ExtendedDisabled(_))
        ));
        assert!(matches!(
            vault.recover(std::slice::from_ref(&pool)).await,
            Err(TornadoVaultError::ExtendedDisabled(_))
        ));
        assert!(matches!(
            vault
                .withdrawal_inputs(&note(pool, Some(0), false), Address::ZERO)
                .await,
            Err(TornadoVaultError::ExtendedDisabled(_))
        ));
        assert!(vault.db.tracked_pools().await.expect("tracked").is_empty());
        assert_eq!(vault.db.next_nonce().await.expect("nonce"), 0);
    }

    #[tokio::test]
    async fn enabled_extended_deposit_is_tracked_and_survives_reopening_with_support_disabled() {
        let mut vault = offline_vault(PoolSelection::IncludeExtended);
        let pool = Pool::SEPOLIA_ETHER_003;
        let calls = vault.deposit(&pool).await.expect("deposit");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target, pool.address);
        assert_eq!(calls[0].value, U256::from(pool.amount_wei));
        assert_eq!(
            vault.db.tracked_pools().await.expect("tracked").as_slice(),
            std::slice::from_ref(&pool)
        );
        assert_eq!(
            vault
                .pool_notes(std::slice::from_ref(&pool))
                .await
                .expect("notes")
                .len(),
            1
        );
        vault.selection = PoolSelection::Classic;
        // Turning support off must not delete stored notes or tracked pools.
        assert_eq!(vault.db.tracked_pools().await.expect("tracked"), [pool]);
    }

    #[tokio::test]
    async fn configured_note_selection_excludes_extended_without_deleting_stored_notes() {
        use alloy_primitives::U64;
        use alloy_transport::mock::Asserter;
        use kohaku_tornadocash::event::Snapshot;
        use kohaku_tornadocash_syncer::SyncerError;
        struct EmptySyncer;
        #[async_trait::async_trait]
        impl Syncer for EmptySyncer {
            async fn sync_range(
                &self,
                pool: &Pool,
                from: u64,
                to: u64,
            ) -> Result<Snapshot, SyncerError> {
                Ok(
                    Snapshot::new(pool.clone(), from..to, Vec::new())
                        .expect("valid empty snapshot"),
                )
            }
        }
        let mock = Asserter::new();
        let mut vault = TornadoVault::with_syncer(
            PrivateKeySigner::random(),
            Arc::new(crate::database::memory::MemoryDatabase::default()),
            crate::test_support::mocked_provider(&mock),
            DynSyncer::new(EmptySyncer),
        )
        .with_pool_selection(PoolSelection::IncludeExtended);
        let pool = Pool::ETHEREUM_ETHER_003;
        vault.deposit(&pool).await.expect("deposit");
        let db = vault.pool_db(&pool);
        let mut records = db.notes().await.expect("records");
        records[0].leaf_index = Some(0);
        db.put_notes(&records).await.expect("mined");
        let amount = U256::from(pool.amount_wei);
        for enabled in [true, false, true] {
            vault.selection = if enabled {
                PoolSelection::IncludeExtended
            } else {
                PoolSelection::Classic
            };
            for cover in [false, true] {
                mock.push_success(&U64::from(1));
                mock.push_success(&U64::from(pool.deployed_block + 1));
                let selected = if cover {
                    vault.cover_notes(amount).await
                } else {
                    vault.exact_notes(amount).await
                };
                assert_eq!(selected.is_ok(), enabled);
            }
            assert_eq!(db.notes().await.expect("records").len(), 1);
            assert_eq!(
                vault.db.tracked_pools().await.expect("tracked").as_slice(),
                std::slice::from_ref(&pool)
            );
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
