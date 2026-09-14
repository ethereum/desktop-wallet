use std::sync::Arc;

use alloy_primitives::{Address, U256};
use futures::lock::Mutex;
use kohaku_tornadocash::{
    circuit::Circuit,
    indexer::rpc::RpcSyncer,
    provider::{
        note::Note,
        pool::{Asset, POOLS, Pool},
        tornado_provider::TornadoProvider,
    },
};
use rand::CryptoRng;

use crate::{
    asset::AssetId,
    call::Call,
    database::{Database, scoped::ScopedDatabaseExt},
    network::{SimpleNetworkEndpoint, endpoint::NetworkEndpoint},
    vault::{
        Vault, VaultError, VaultId,
        tornadocash::{db::TcVaultDb, kv_adapter::KvAdapter},
    },
};

mod db;
mod kv_adapter;

const TORNADOCASH_VAULT_TAG: &str = "tornadocash";
const TORNADOCASH_KV_SCOPE: &[u8] = b"kohaku_tornadocash_kvstore";

pub struct TcVault {
    network_id: u64,

    provider: Arc<Mutex<TornadoProvider>>,
    db: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum TcVaultError {
    #[error("Invalid asset: {0:?}")]
    InvalidAsset(AssetId),
    #[error("Invalid amount: {0}")]
    InvalidAmount(U256),
    #[error("No notes found for pool: {0:?}")]
    NoNotesForPool(Pool),
    #[error("Circuit error: {0}")]
    Circuit(#[from] kohaku_tornadocash::circuit::CircuitError),
    #[error("TC provider error: {0}")]
    Tc(#[from] kohaku_tornadocash::provider::tornado_provider::TornadoProviderError),
    #[error("Transport error: {0}")]
    Transport(#[from] alloy_transport::TransportError),
    #[error("Database error: {0}")]
    Database(#[from] crate::database::DatabaseError),
}

impl TcVault {
    pub async fn new(
        provider: SimpleNetworkEndpoint,
        db: Arc<dyn Database>,
    ) -> Result<Self, TcVaultError> {
        let network_id = provider.network_id().await?;
        let provider = provider.provider;
        let syncer = RpcSyncer::new(provider.clone());

        let store = KvAdapter(db.clone().scoped(TORNADOCASH_KV_SCOPE));
        let circuit = Circuit::from_remote().await?;

        let tornado_provider = TornadoProvider::new(
            provider.clone(),
            store.into(),
            syncer.clone().into(),
            syncer.into(),
            circuit,
        );

        Ok(Self {
            network_id,
            provider: Arc::new(Mutex::new(tornado_provider)),
            db,
        })
    }
}

#[async_trait::async_trait]
impl Vault for TcVault {
    fn tag(&self) -> &'static str {
        TORNADOCASH_VAULT_TAG
    }

    fn id(&self) -> VaultId {
        VaultId::Other {
            tag: TORNADOCASH_VAULT_TAG.to_string(),
            id: "default".to_string(),
        }
    }

    async fn balance(&self, asset: &AssetId) -> Result<U256, VaultError> {
        //? Silently ignore invalid asset IDs, since returning 0 is more ideomatic
        let notes = self.pools_for_asset(asset).await.unwrap_or(Vec::new());
        let total_balance: U256 = notes.iter().map(|n| U256::from(n.amount_wei)).sum();

        Ok(total_balance)
    }
}

impl TcVault {
    /// Deposits into the given tornadocash pool.
    pub async fn deposit(
        &self,
        pool: Pool,
        rng: &mut impl CryptoRng,
    ) -> Result<Vec<Call>, TcVaultError> {
        let (call, note) = self.provider.lock().await.deposit(pool, rng)?;
        self.db.append_note(note).await?;

        Ok(vec![Call::new(call.target, call.data, call.value)])
    }

    /// Withdraws from the given tornadocash pool to the given address.
    pub async fn withdraw(
        &self,
        to: Address,
        note: Pool,
        rng: &mut impl CryptoRng,
    ) -> Result<Vec<Call>, TcVaultError> {
        let notes = self.db.notes().await?;
        let note = notes
            .iter()
            .find(|n| Pool::from_note(n) == Some(note))
            .ok_or_else(|| TcVaultError::NoNotesForPool(note))?;

        let call = self
            .provider
            .lock()
            .await
            .withdraw(note, to, None, None, None, rng)
            .await?;

        Ok(vec![Call::new(call.target, call.data, call.value)])
    }

    /// Returns all notes managed by this vault.
    pub async fn notes(&self) -> Result<Vec<Note>, TcVaultError> {
        Ok(self.db.notes().await?)
    }

    async fn pools_for_asset(&self, asset: &AssetId) -> Result<Vec<Pool>, TcVaultError> {
        let chain_id = self.network_id;
        let asset = asset_to_tc_asset(self.network_id, asset)?;

        let pools: Vec<_> = self
            .db
            .notes()
            .await?
            .iter()
            .filter_map(Pool::from_note)
            .filter(|n| n.chain_id == chain_id)
            .filter(|n| n.asset == asset)
            .collect();

        Ok(pools)
    }
}

impl From<TcVaultError> for VaultError {
    fn from(err: TcVaultError) -> Self {
        VaultError::Other(Box::new(err))
    }
}

/// Returns the tornadocash asset corresponding to a given `asset` on a given
/// `network_id`, or an error if no such asset exists.
#[expect(
    clippy::result_large_err,
    reason = "internal-only fn allowed large error type"
)]
fn asset_to_tc_asset(network_id: u64, asset: &AssetId) -> Result<Asset, TcVaultError> {
    POOLS
        .iter()
        .find(|p| p.chain_id == network_id && tc_asset_matches(&p.asset, asset))
        .map(|p| p.asset)
        .ok_or_else(|| TcVaultError::InvalidAsset(asset.clone()))
}

fn tc_asset_matches(tc_asset: &Asset, asset: &AssetId) -> bool {
    match (tc_asset, asset) {
        (Asset::Native { .. }, AssetId::Native) => true,
        (Asset::Erc20 { address, .. }, AssetId::Erc20(a)) => address == a,
        _ => false,
    }
}
