//! Native Railgun vault. All spending and fee selection goes through `PoiWallet`.
use std::sync::Arc;

use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner as AccountSigner;
use alloy_sol_types::{SolCall, sol};
use kohaku_railgun::{
    account::{
        address::RailgunAddress,
        chain::ChainId,
        signer::{PrivateKeySigner, RailgunSigner},
    },
    builder::RailgunBuilder,
    caip::AssetId as RailgunAsset,
    chain_config::ChainConfig,
    crypto::keys::{ByteKey, SpendingKey, ViewingKey},
    poi::types::PoiStatus,
    transact::{ShieldBuilder, Spend},
    wallet::NoteEntry,
};
pub use kohaku_railgun_poi::provider::PoiProcessReport as PoiReport;
use kohaku_railgun_poi::{
    provider::PoiProcessReport,
    wallet::{PoiWallet, PoiWalletCheckpoint},
};
use railgun_userop_kit::{
    bundler::Bundler,
    signed_user_operation::SignedUserOperation,
    smart_account::simple_7702_smart_account::{Call as AccountCall, Simple7702SmartAccount},
    user_operation::UserOperationGasEstimate,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    asset::AssetId,
    call::Call,
    database::{Database, DatabaseGuard},
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    vault::{Balance, Vault, VaultError, VaultId, Visibility},
};

pub mod bundler;
mod decode;
#[cfg(test)]
mod quote_tests;
mod rpc_operations;
pub mod syncer;

const CHECKPOINT: &[u8] = b"checkpoint-v1";
const SYNC_WINDOW: u64 = 250_000;

pub type Result<T> = std::result::Result<T, RailgunError>;

sol! { function approve(address spender, uint256 amount) external returns (bool); function withdraw(uint256 amount) external; }

/// No keys are serialized. This record is stored inside EDW's encrypted database.
#[derive(Serialize, Deserialize)]
struct StoredState {
    version: u32,
    chain_id: u64,
    address: RailgunAddress,
    wallet: PoiWalletCheckpoint,
    pending: Option<PendingOperation>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingOperation {
    pub signed: SignedUserOperation,
    pub fee: U256,
    /// Retained for crash recovery before the fresh-address index was committed.
    pub fresh_index: u32,
}

/// The profile's account and configuration. A session holds the database lock
/// across read/modify/write, proving and submission, including across processes.
#[derive(Clone)]
pub struct RailgunVault {
    db: Arc<dyn Database>,
    endpoint: Arc<dyn NetworkEndpoint>,
    chain: ChainConfig,
    signer: Arc<PrivateKeySigner>,
    saga_url: Option<String>,
    artifact_base_url: Option<String>,
}

pub struct RailgunSession<'a> {
    vault: &'a RailgunVault,
    wallet: PoiWallet,
    pending: Option<PendingOperation>,
    _guard: DatabaseGuard,
    observed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RailgunNote {
    pub token: String,
    pub amount: String,
    pub poi_status: String,
    pub tree: u32,
    pub leaf: u32,
    pub memo: String,
}

pub enum Movement {
    Unshield {
        token: Address,
        amount: U256,
        unwrap_native: bool,
        calls: Vec<Call>,
    },
    Transfer {
        token: Address,
        amount: U256,
        recipient: RailgunAddress,
        memo: String,
    },
}

/// A simulation with dummy proofs. No spending or account signature was requested.
pub struct RailgunQuote {
    pub fee: U256,
    pub protocol_fee: U256,
    spends: Vec<Spend>,
    calls: Vec<AccountCall>,
    gas: UserOperationGasEstimate,
    owner: Address,
}

#[derive(Debug, thiserror::Error)]
pub enum RailgunError {
    #[error("Railgun: {0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] crate::database::DatabaseError),
    #[error(transparent)]
    Network(#[from] crate::network::NetworkEndpointError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Abi(#[from] alloy_sol_types::Error),
    #[error(transparent)]
    Wallet(Box<kohaku_railgun::wallet::RailgunError>),
    #[error(transparent)]
    Poi(Box<kohaku_railgun_poi::wallet::PoiWalletError>),
    #[error(transparent)]
    UserOp(Box<kohaku_railgun_paymaster::UserOpError>),
    #[error(transparent)]
    Bundler(#[from] railgun_userop_kit::bundler::BundlerError),
    #[error(transparent)]
    Mnemonic(#[from] crate::mnemonic::MnemonicError),
    #[error("Railgun operation {0} is pending; use `edw railgun retry` before spending again")]
    Pending(String),
}

impl From<kohaku_railgun::wallet::RailgunError> for RailgunError {
    fn from(error: kohaku_railgun::wallet::RailgunError) -> Self {
        Self::Wallet(Box::new(error))
    }
}
impl From<kohaku_railgun_poi::wallet::PoiWalletError> for RailgunError {
    fn from(error: kohaku_railgun_poi::wallet::PoiWalletError) -> Self {
        Self::Poi(Box::new(error))
    }
}
impl From<kohaku_railgun_paymaster::UserOpError> for RailgunError {
    fn from(error: kohaku_railgun_paymaster::UserOpError) -> Self {
        Self::UserOp(Box::new(error))
    }
}

impl RailgunQuote {
    #[must_use]
    pub fn execution_calls(&self) -> Vec<Call> {
        self.calls
            .iter()
            .map(|c| Call::new(c.target, c.data.clone(), c.value))
            .collect()
    }
}

impl RailgunVault {
    pub fn open(
        mnemonic: &Mnemonic,
        profile_index: u32,
        chain_id: u64,
        db: Arc<dyn Database>,
        endpoint: Arc<dyn NetworkEndpoint>,
        saga_url: Option<&str>,
    ) -> Result<Self> {
        let chain = ChainConfig::from_chain_id(chain_id).ok_or_else(|| {
            RailgunError::Invalid(format!("unsupported Railgun chain {chain_id}"))
        })?;
        let (spending, viewing) = mnemonic.railgun_keys(profile_index)?;
        let spending = Zeroizing::new(spending);
        let viewing = Zeroizing::new(viewing);
        let signer = PrivateKeySigner::new_evm(
            SpendingKey::from_bytes(*spending),
            ViewingKey::from_bytes(*viewing),
            chain_id,
        );
        Ok(Self {
            db,
            endpoint,
            chain,
            signer,
            saga_url: saga_url.map(str::to_string),
            artifact_base_url: None,
        })
    }

    #[must_use]
    pub fn with_artifact_base_url(mut self, url: String) -> Self {
        self.artifact_base_url = Some(url);
        self
    }
    #[must_use]
    pub fn address(&self) -> RailgunAddress {
        self.signer.address()
    }
    #[must_use]
    pub fn chain(&self) -> &ChainConfig {
        &self.chain
    }

    pub async fn session(&self) -> Result<RailgunSession<'_>> {
        self.endpoint
            .verify_network_id(crate::network::NetworkId(self.chain.id))
            .await?;
        let guard = self.db.lock().await?;
        let (state, pending) = if let Some(bytes) = self.db.get(CHECKPOINT).await? {
            let stored: StoredState = serde_json::from_slice(&bytes)?;
            if stored.version != 1
                || stored.chain_id != self.chain.id
                || stored.address != self.address()
            {
                return Err(RailgunError::Invalid(
                    "checkpoint version, chain or profile mismatch".into(),
                ));
            }
            (stored.wallet, stored.pending)
        } else {
            (PoiWalletCheckpoint::default(), None)
        };
        let provider = self.endpoint.provider().ok_or_else(|| {
            RailgunError::Invalid("Railgun requires an Alloy RPC provider".into())
        })?;
        let syncer = Arc::new(syncer::RailgunSyncer::new(
            self.chain.clone(),
            self.endpoint.clone(),
            self.saga_url.as_deref(),
        )?);
        let mut builder = RailgunBuilder::new(self.chain.clone(), provider)
            .with_state(state.railgun)
            .with_utxo_syncer(syncer.clone());
        if let Some(url) = &self.artifact_base_url {
            builder = builder.with_artifact_base_url(url);
        }
        let mut wallet = PoiWallet::new(builder.build().await?, state.poi, syncer)?;
        wallet.register(self.signer.clone()).await?;
        Ok(RailgunSession {
            vault: self,
            wallet,
            pending,
            _guard: guard,
            observed: false,
        })
    }

    /// Native shielding wraps ETH through `RelayAdapt`. ERC-20 approvals use the
    /// exact shield amount and precede the shield call.
    pub fn deposit(&self, asset: &AssetId, amount: U256) -> Result<Vec<Call>> {
        let amount = note_amount(amount)?;
        let mut builder = ShieldBuilder::new(self.chain.clone());
        let mut calls = Vec::new();
        match asset {
            AssetId::Native => {
                builder = builder.shield_native(self.address(), amount);
            }
            AssetId::Erc20(token) => {
                calls.push(Call::new(
                    *token,
                    approveCall::new((self.chain.railgun_smart_wallet, U256::from(amount)))
                        .abi_encode()
                        .into(),
                    U256::ZERO,
                ));
                builder = builder.shield(self.address(), RailgunAsset::Erc20(*token), amount);
            }
        }
        let transactions = builder
            .build(&mut rand::rng())
            .map_err(|e| RailgunError::Invalid(e.to_string()))?;
        for tx in transactions {
            let target = tx
                .to
                .and_then(alloy_primitives::TxKind::into_to)
                .ok_or_else(|| RailgunError::Invalid("shield call without target".into()))?;
            calls.push(Call::new(
                target,
                tx.input.into_input().unwrap_or_default(),
                tx.value.unwrap_or_default(),
            ));
        }
        Ok(calls)
    }
}

impl RailgunSession<'_> {
    pub async fn persist(&self) -> Result<()> {
        let state = StoredState {
            version: 1,
            chain_id: self.vault.chain.id,
            address: self.vault.address(),
            wallet: self.wallet.state(),
            pending: self.pending.clone(),
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&state)?);
        self.vault.db.put(CHECKPOINT, &bytes).await?;
        Ok(())
    }

    /// Persist each completed window and the recovered PPOI queue before
    /// submission. An accepted submission is not treated as locally Valid.
    pub async fn sync(&mut self, submit_poi: bool) -> Result<Option<PoiProcessReport>> {
        self.observed = false;
        let tip = self.vault.endpoint.block_height().await?;
        loop {
            let next = self
                .wallet
                .synced_block()
                .max(self.vault.chain.deployment_block.saturating_sub(1))
                .saturating_add(SYNC_WINDOW)
                .min(tip);
            let result = self.wallet.sync_to_without_poi_submission(next).await;
            self.persist().await?;
            result?;
            tracing::info!(
                block = self.wallet.synced_block(),
                tip,
                "Railgun checkpoint saved"
            );
            if next >= tip {
                break;
            }
        }
        if !submit_poi {
            self.observed = true;
            return Ok(None);
        }
        let report = self.wallet.process_pending_poi_report().await;
        self.persist().await?;
        // Refresh validated lists after submission, without claiming immediate
        // acceptance or repeatedly re-submitting waiting proofs.
        if !report.submitted.is_empty() {
            let result = self.wallet.sync_to_without_poi_submission(tip).await;
            self.persist().await?;
            result?;
        }
        self.observed = true;
        Ok(Some(report))
    }

    pub async fn notes(&mut self) -> Vec<RailgunNote> {
        self.wallet
            .notes(self.vault.address())
            .await
            .into_iter()
            .map(|note| RailgunNote {
                token: note.asset.to_string(),
                amount: note.amount.to_string(),
                poi_status: note
                    .poi_status
                    .map_or_else(|| "Unavailable".into(), |s| s.to_string()),
                tree: note.tree_number,
                leaf: note.leaf_index,
                memo: note.memo,
            })
            .collect()
    }

    pub async fn balance(&mut self, asset: &AssetId) -> Balance {
        let token = match asset {
            AssetId::Native => return Balance::default(),
            AssetId::Erc20(token) => *token,
        };
        note_balance(self.wallet.notes(self.vault.address()).await, token)
    }

    #[must_use]
    pub fn pending(&self) -> Option<&PendingOperation> {
        self.pending.as_ref()
    }
    #[must_use]
    pub fn poi_pending(&self) -> Vec<kohaku_railgun_poi::provider::PendingPoiSummary> {
        self.wallet.poi_pending()
    }

    #[allow(clippy::too_many_lines)]
    pub async fn quote(
        &mut self,
        movement: Movement,
        owner: &AccountSigner,
        bundler: &dyn Bundler,
    ) -> Result<RailgunQuote> {
        if let Some(pending) = &self.pending {
            return Err(RailgunError::Pending(
                pending.signed.hash(self.vault.chain.id).0.to_string(),
            ));
        }
        if !self.observed {
            return Err(RailgunError::Invalid(
                "sync Railgun and PPOI before quoting a spend".into(),
            ));
        }
        let signer: Arc<dyn RailgunSigner> = self.vault.signer.clone();
        let mut rng = rand::rng();
        let (spends, calls, protocol_fee) = match movement {
            Movement::Transfer {
                token,
                amount,
                recipient,
                memo,
            } => {
                if recipient.chain() != ChainId::All
                    && recipient.chain() != ChainId::evm(self.vault.chain.id)
                {
                    return Err(RailgunError::Invalid(
                        "recipient Railgun address names another chain".into(),
                    ));
                }
                let spends = self
                    .wallet
                    .transactions()
                    .transfer_amount(
                        signer.clone(),
                        recipient,
                        RailgunAsset::Erc20(token),
                        note_amount(amount)?,
                        &memo,
                        &mut rng,
                    )
                    .await?;
                (spends, Vec::new(), U256::ZERO)
            }
            Movement::Unshield {
                token,
                amount,
                unwrap_native,
                calls,
            } => {
                let net = note_amount(amount)?;
                let gross = gross_up(net, self.vault.chain.unshield_fee_bps)?;
                let calls = execution_calls(
                    self.vault.chain.wrapped_base_token,
                    token,
                    net,
                    unwrap_native,
                    calls,
                )?;
                let spends = self
                    .wallet
                    .transactions()
                    .unshield_amount(
                        signer.clone(),
                        owner.address(),
                        RailgunAsset::Erc20(token),
                        gross,
                        &mut rng,
                    )
                    .await?;
                (spends, calls, U256::from(gross - net))
            }
        };
        let provider = self
            .vault
            .endpoint
            .provider()
            .ok_or_else(|| RailgunError::Invalid("missing provider".into()))?;
        let account =
            Simple7702SmartAccount::new(provider.clone(), owner.address(), self.vault.chain.id);
        let gas = self
            .wallet
            .user_operations(provider)
            .simulate_gas_limits(
                &spends,
                bundler,
                &account,
                signer,
                self.vault.chain.wrapped_base_token,
                &calls,
                &mut rng,
            )
            .await?;
        let total = gas
            .pre_verification_gas
            .checked_add(gas.verification_gas_limit)
            .and_then(|v| v.checked_add(gas.call_gas_limit))
            .and_then(|v| v.checked_add(gas.paymaster_verification_gas_limit.unwrap_or_default()))
            .and_then(|v| v.checked_add(gas.paymaster_post_op_gas_limit.unwrap_or_default()))
            .ok_or_else(|| RailgunError::Invalid("gas overflow".into()))?;
        let fee = U256::from(total) * U256::from(gas.max_fee_per_gas);
        Ok(RailgunQuote {
            fee,
            protocol_fee,
            spends,
            calls,
            gas,
            owner: owner.address(),
        })
    }

    /// The durable journal is written before the signed operation can reach a
    /// bundler. A retry submits these exact bytes, never reselects notes.
    pub async fn prepare(
        &mut self,
        quote: RailgunQuote,
        owner: &AccountSigner,
        fresh_index: u32,
        bundler: &dyn Bundler,
    ) -> Result<PendingOperation> {
        if let Some(pending) = &self.pending {
            return Err(RailgunError::Pending(
                pending.signed.hash(self.vault.chain.id).0.to_string(),
            ));
        }
        if owner.address() != quote.owner {
            return Err(RailgunError::Invalid("quote account changed".into()));
        }
        let provider = self
            .vault
            .endpoint
            .provider()
            .ok_or_else(|| RailgunError::Invalid("missing provider".into()))?;
        let account =
            Simple7702SmartAccount::new(provider.clone(), owner.address(), self.vault.chain.id);
        let prepared = self
            .wallet
            .user_operations(provider)
            .prepare_userop_single_proof(
                &quote.spends,
                bundler,
                &account,
                self.vault.signer.clone(),
                self.vault.chain.wrapped_base_token,
                quote.calls,
                quote.gas,
                &mut rand::rng(),
            )
            .await;
        self.persist().await?;
        let (signable, report) = prepared?;
        let signed = signable
            .sign(owner)
            .await
            .map_err(|e| RailgunError::Invalid(e.to_string()))?;
        let pending = PendingOperation {
            signed,
            fee: U256::from(report.fee),
            fresh_index,
        };
        self.pending = Some(pending.clone());
        self.persist().await?;
        Ok(pending)
    }

    /// Acceptance alone leaves the journal in place. Clear it only after an
    /// inclusion receipt, then resync before any further spend.
    pub async fn submit_pending(&mut self, bundler: &dyn Bundler) -> Result<(String, String)> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| RailgunError::Invalid("no pending Railgun operation".into()))?;
        let expected = pending.signed.hash(self.vault.chain.id);
        let receipt = if let Some(receipt) = bundler.receipt(expected).await? {
            receipt
        } else {
            let hash = bundler.send_user_operation(&pending.signed).await?;
            if hash != expected {
                return Err(RailgunError::Invalid(
                    "bundler returned a different UserOperation hash".into(),
                ));
            }
            bundler.wait_for_receipt(hash).await?
        };
        if receipt.user_op_hash != expected.0
            || receipt.sender != pending.signed.user_op.sender
            || receipt.entry_point != pending.signed.entry_point
        {
            return Err(RailgunError::Invalid(
                "bundler receipt does not match the pending operation".into(),
            ));
        }
        let hash = expected;
        self.observed = false;
        self.pending = None;
        self.persist().await?;
        if !receipt.success {
            return Err(RailgunError::Invalid(format!(
                "operation {} reverted in {}",
                hash.0, receipt.receipt.transaction_hash
            )));
        }
        Ok((
            hash.0.to_string(),
            receipt.receipt.transaction_hash.to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl Vault for RailgunVault {
    fn tag(&self) -> &'static str {
        "railgun"
    }
    fn id(&self) -> VaultId {
        VaultId::Railgun(self.address().to_string())
    }
    fn visibility(&self) -> Visibility {
        Visibility::Private
    }
    async fn balance(&self, asset: &AssetId) -> std::result::Result<Balance, VaultError> {
        let vault = self.clone();
        let asset = asset.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let mut session = vault.session().await?;
                session.sync(true).await?;
                Ok::<_, RailgunError>(session.balance(&asset).await)
            })
        })
        .await
        .map_err(|e| VaultError::Other(Box::new(e)))?
        .map_err(|e| VaultError::Other(Box::new(e)))
    }
}

fn note_balance(notes: Vec<NoteEntry>, token: Address) -> Balance {
    let mut result = Balance::default();
    for note in notes
        .into_iter()
        .filter(|n| n.asset == RailgunAsset::Erc20(token))
    {
        if note.poi_status == Some(PoiStatus::Valid) {
            result.spendable = result.spendable.saturating_add(U256::from(note.amount));
        } else {
            result.pending = result.pending.saturating_add(U256::from(note.amount));
        }
    }
    result
}

fn note_amount(amount: U256) -> Result<u128> {
    if amount.is_zero() || amount >= U256::from(1u128 << 120) {
        return Err(RailgunError::Invalid(
            "amount must be positive and fit Railgun's 120-bit note limit".into(),
        ));
    }
    Ok(amount.to::<u128>())
}

pub fn gross_up(net: u128, fee_bps: u16) -> Result<u128> {
    let divisor = 10_000u16
        .checked_sub(fee_bps)
        .filter(|v| *v > 0)
        .ok_or_else(|| RailgunError::Invalid("invalid unshield fee".into()))?;
    if net == 0 {
        return Ok(0);
    }
    let gross = U256::from(net - 1) * U256::from(10_000) / U256::from(divisor) + U256::from(1);
    note_amount(gross)
}

fn execution_calls(
    weth: Address,
    token: Address,
    amount: u128,
    unwrap: bool,
    calls: Vec<Call>,
) -> Result<Vec<AccountCall>> {
    if unwrap && token != weth {
        return Err(RailgunError::Invalid(
            "native unshield requires WETH".into(),
        ));
    }
    let mut out = Vec::new();
    if unwrap {
        out.push(AccountCall {
            target: weth,
            value: U256::ZERO,
            data: withdrawCall::new((U256::from(amount),)).abi_encode().into(),
        });
    }
    out.extend(calls.into_iter().map(|c| AccountCall {
        target: c.target,
        value: c.value,
        data: c.data,
    }));
    Ok(out)
}

#[cfg(test)]
mod tests;
