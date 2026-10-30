use std::{fmt, sync::Arc};

use alloy_primitives::U256;
use clap::Args;
use edw_core::{
    asset::{AssetId, NATIVE_DECIMALS, format_amount},
    balance::{self, AssetTotals, VaultBalances},
    factory::FactoryError,
    instance::Instance,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    vault::{
        Balance, Vault, Visibility,
        db::VaultRecord,
        tornadocash::{UnspentNote, unspent_holdings},
    },
};
use serde::Serialize;

use crate::{
    GlobalArgs,
    profile::{ProfileSummary, choose_profile},
    report::Report,
};

#[derive(Args, Debug)]
pub struct BalanceArgs {
    /// Profile as `mnemonic/profile` or a unique name. Prompted for when more than one exists.
    selector: Option<String>,
    /// Also list each vault's balance and each unspent Tornado note.
    #[arg(long)]
    detailed: bool,
}

#[derive(Clone, Serialize)]
struct Asset {
    symbol: String,
    address: Option<String>,
    decimals: u8,
    #[serde(skip)]
    id: AssetId,
}

#[derive(Serialize)]
struct Amount {
    #[serde(skip)]
    value: U256,
    #[serde(rename = "amount")]
    base_units: String,
    formatted: String,
}

#[derive(Serialize)]
struct BalanceAmounts {
    spendable: Amount,
    pending: Amount,
}

#[derive(Serialize)]
struct AssetRow {
    #[serde(flatten)]
    asset: Asset,
    public: BalanceAmounts,
    private: BalanceAmounts,
    total: BalanceAmounts,
}

#[derive(Serialize)]
struct VaultAssetRow {
    #[serde(flatten)]
    asset: Asset,
    #[serde(flatten)]
    balance: BalanceAmounts,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum VaultStatus {
    Ok,
    Unavailable,
}

/// A vault whose factory failed, so its id and visibility are unknown.
struct UnbuiltVault {
    record: VaultRecord,
    error: FactoryError,
}

#[derive(Serialize)]
struct VaultRow {
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vault: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<Visibility>,
    status: VaultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    assets: Vec<VaultAssetRow>,
}

#[derive(Serialize)]
struct TornadoNoteRow {
    pool: String,
    amount: String,
    #[serde(skip_serializing_if = "is_false")]
    pending: bool,
}

#[derive(Serialize)]
struct BalanceReport {
    profile: ProfileSummary,
    complete: bool,
    assets: Vec<AssetRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vaults: Option<Vec<VaultRow>>,
    /// Unspent Tornado notes. Present only with `--detailed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tornado_notes: Option<Vec<TornadoNoteRow>>,
}

impl Asset {
    fn native(symbol: String) -> Self {
        Self {
            symbol,
            address: None,
            decimals: NATIVE_DECIMALS,
            id: AssetId::Native,
        }
    }

    fn amount(&self, amount: U256) -> anyhow::Result<Amount> {
        Ok(Amount {
            value: amount,
            base_units: amount.to_string(),
            formatted: format_amount(amount, self.decimals)?,
        })
    }

    fn balance(&self, balance: Balance) -> anyhow::Result<BalanceAmounts> {
        Ok(BalanceAmounts {
            spendable: self.amount(balance.spendable)?,
            pending: self.amount(balance.pending)?,
        })
    }

    fn row(&self, totals: &AssetTotals) -> anyhow::Result<AssetRow> {
        Ok(AssetRow {
            asset: self.clone(),
            public: self.balance(totals.public)?,
            private: self.balance(totals.private)?,
            total: self.balance(totals.total())?,
        })
    }
}

impl BalanceArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let profile = if let Some(selector) = &self.selector {
            instance.profile(selector).await?
        } else {
            let mut profiles = instance.profiles().await?;
            profiles.sort_by_key(ProfileRecord::key);
            choose_profile(&profiles, global.input())?
        };

        let erc20s = instance.profile_erc20s(&profile).await?;
        let assets: Vec<Asset> =
            std::iter::once(Asset::native(instance.network().native_asset.clone()))
                .chain(erc20s.into_iter().map(|erc20| Asset {
                    symbol: erc20.symbol,
                    address: Some(erc20.address.to_string()),
                    decimals: erc20.decimals,
                    id: AssetId::Erc20(erc20.address),
                }))
                .collect();
        let ids: Vec<AssetId> = assets.iter().map(|asset| asset.id.clone()).collect();

        let endpoint = instance.rpc().await?;
        let (vaults, unbuilt) = build_vaults(&instance, &profile, &endpoint).await?;
        for vault in &unbuilt {
            eprintln!(
                "Could not build {} vault: {}",
                vault.record.tag, vault.error
            );
        }
        let read = balance::read(&vaults, &ids).await;
        for vault in &read {
            if let Err(error) = &vault.balances {
                eprintln!("Could not read {} {}: {error}", vault.tag, vault.vault);
            }
        }

        let tornado = read_tornado(&instance, &profile, endpoint).await;
        let (holdings, tornado_ok) = match tornado {
            Ok(holdings) => (holdings, true),
            Err(error) => {
                eprintln!("Could not read Tornado notes: {error:#}");
                (Vec::new(), false)
            }
        };
        let vaults = self
            .detailed
            .then(|| vault_rows(&assets, &read, &unbuilt))
            .transpose()?;
        let holdings: Vec<UnspentNote> = holdings
            .into_iter()
            .filter(|note| assets.iter().any(|asset| asset.id == note.asset))
            .collect();
        let mut totals = balance::totals(&read, assets.len());
        fold_tornado(&assets, &mut totals, &holdings);
        let tornado_notes = (self.detailed && tornado_ok)
            .then(|| note_rows(&holdings))
            .transpose()?;

        BalanceReport {
            profile: ProfileSummary::load(&instance, &profile).await?,
            complete: tornado_ok
                && unbuilt.is_empty()
                && read.iter().all(|vault| vault.balances.is_ok()),
            assets: assets
                .iter()
                .zip(&totals)
                .map(|(asset, totals)| asset.row(totals))
                .collect::<anyhow::Result<_>>()?,
            vaults,
            tornado_notes,
        }
        .emit(global.mode())
    }
}

impl fmt::Display for BalanceAmounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.spendable.formatted)?;
        if !self.pending.value.is_zero() {
            write!(f, " (+{} pending)", self.pending.formatted)?;
        }
        Ok(())
    }
}

impl Report for BalanceReport {
    const KIND: &'static str = "edw/balance";
    const VERSION: u32 = 3;
}

impl fmt::Display for BalanceReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Balance of {}", self.profile)?;
        let rows: Vec<Vec<String>> = self
            .assets
            .iter()
            .map(|row| {
                vec![
                    row.asset.symbol.clone(),
                    row.public.to_string(),
                    row.private.to_string(),
                    row.total.to_string(),
                ]
            })
            .collect();
        write_table(f, "", &["asset", "public", "private", "total"], &rows)?;
        if !self.complete {
            write!(f, "\n\nTotals leave out holdings that could not be read.")?;
        }

        if let Some(vaults) = &self.vaults {
            if vaults.is_empty() {
                write!(f, "\n\nNo vaults.")?;
            }
            for vault in vaults {
                write!(f, "\n\n{}", vault.tag)?;
                if let (Some(id), Some(visibility)) = (&vault.vault, &vault.visibility) {
                    write!(f, " {id} ({visibility})")?;
                }
                if let Some(error) = &vault.error {
                    write!(f, "\n  unavailable: {error}")?;
                    continue;
                }
                let rows: Vec<Vec<String>> = vault
                    .assets
                    .iter()
                    .map(|row| vec![row.asset.symbol.clone(), row.balance.to_string()])
                    .collect();
                write_table(f, "  ", &["asset", "balance"], &rows)?;
            }
        }
        if let Some(notes) = &self.tornado_notes {
            write!(f, "{}", tornado_lines(notes))?;
        }
        Ok(())
    }
}

/// Builds `profile`'s vaults. An empty record list needs no endpoint.
async fn build_vaults(
    instance: &Instance,
    profile: &ProfileRecord,
    endpoint: &Arc<dyn NetworkEndpoint>,
) -> anyhow::Result<(Vec<Box<dyn Vault>>, Vec<UnbuiltVault>)> {
    let records = instance.vault_records(profile).await?;
    if records.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let built = instance.build_vaults(profile, &records, endpoint).await;

    let mut vaults = Vec::new();
    let mut unbuilt = Vec::new();
    for (record, vault) in records.into_iter().zip(built) {
        match vault {
            Ok(vault) => vaults.push(vault),
            Err(error) => unbuilt.push(UnbuiltVault { record, error }),
        }
    }
    Ok((vaults, unbuilt))
}

fn vault_rows(
    assets: &[Asset],
    read: &[VaultBalances],
    unbuilt: &[UnbuiltVault],
) -> anyhow::Result<Vec<VaultRow>> {
    let mut rows = read
        .iter()
        .map(|vault| {
            let (status, error, rows) = match &vault.balances {
                Ok(balances) => (VaultStatus::Ok, None, vault_asset_rows(assets, balances)?),
                Err(error) => (
                    VaultStatus::Unavailable,
                    Some(error.to_string()),
                    Vec::new(),
                ),
            };
            Ok(VaultRow {
                tag: vault.tag.to_string(),
                vault: Some(vault.vault.to_string()),
                visibility: Some(vault.visibility),
                status,
                error,
                assets: rows,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    rows.extend(unbuilt.iter().map(|vault| VaultRow {
        tag: vault.record.tag.clone(),
        vault: None,
        visibility: None,
        status: VaultStatus::Unavailable,
        error: Some(vault.error.to_string()),
        assets: Vec::new(),
    }));
    Ok(rows)
}

fn vault_asset_rows(assets: &[Asset], balances: &[Balance]) -> anyhow::Result<Vec<VaultAssetRow>> {
    assets
        .iter()
        .zip(balances)
        .map(|(asset, balance)| {
            Ok(VaultAssetRow {
                asset: asset.clone(),
                balance: asset.balance(*balance)?,
            })
        })
        .collect()
}

async fn read_tornado(
    instance: &Instance,
    profile: &ProfileRecord,
    endpoint: Arc<dyn NetworkEndpoint>,
) -> anyhow::Result<Vec<UnspentNote>> {
    let phrase = crate::spend::mnemonic(instance, profile).await?;
    let vault = crate::spend::open_vault(instance, profile, &phrase, endpoint).await?;
    Ok(unspent_holdings(&vault.notes().await?))
}

/// Adds unspent Tornado notes to the private column of an asset the profile already shows.
///
/// A note for any other token is left out. Hiding that token hides its Tornado balance too.
fn fold_tornado(assets: &[Asset], totals: &mut [AssetTotals], notes: &[UnspentNote]) {
    for note in notes {
        let Some(index) = assets.iter().position(|asset| asset.id == note.asset) else {
            continue;
        };
        let side = &mut totals[index].private;
        if note.pending {
            side.pending = side.pending.saturating_add(note.amount_wei);
        } else {
            side.spendable = side.spendable.saturating_add(note.amount_wei);
        }
    }
}

fn note_rows(notes: &[UnspentNote]) -> anyhow::Result<Vec<TornadoNoteRow>> {
    notes
        .iter()
        .map(|note| {
            Ok(TornadoNoteRow {
                pool: note.pool_id.clone(),
                amount: format!(
                    "{} {}",
                    format_amount(note.amount_wei, note.decimals)?,
                    note.symbol
                ),
                pending: note.pending,
            })
        })
        .collect()
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

fn tornado_lines(notes: &[TornadoNoteRow]) -> String {
    if notes.is_empty() {
        return "\n\ntornado\n  no notes".to_string();
    }
    let mut lines = vec!["\n\ntornado".to_string()];
    for note in notes {
        let pending = if note.pending { "  pending" } else { "" };
        lines.push(format!("\n  {}  {}{pending}", note.amount, note.pool));
    }
    lines.concat()
}

fn write_table(
    f: &mut fmt::Formatter<'_>,
    indent: &str,
    header: &[&str],
    rows: &[Vec<String>],
) -> fmt::Result {
    let header: Vec<String> = header.iter().map(ToString::to_string).collect();
    let mut widths: Vec<usize> = header.iter().map(String::len).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    for row in std::iter::once(&header).chain(rows) {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        write!(f, "\n{indent}{}", cells.join("  ").trim_end())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use edw_core::vault::{VaultError, VaultId};

    use super::*;

    fn amount(value: u64, formatted: &str) -> Amount {
        Amount {
            value: U256::from(value),
            base_units: value.to_string(),
            formatted: formatted.to_string(),
        }
    }

    #[test]
    fn pending_shows_beside_spendable() {
        let balance = BalanceAmounts {
            spendable: amount(15, "1.5"),
            pending: amount(1, "0.1"),
        };
        assert_eq!(balance.to_string(), "1.5 (+0.1 pending)");
    }

    #[test]
    fn a_vault_that_failed_to_read_is_unavailable_with_its_error() {
        let read = [VaultBalances {
            tag: "simple-vault",
            vault: VaultId::Address(Address::ZERO),
            visibility: Visibility::Public,
            balances: Err(VaultError::UnsupportedVaultId(VaultId::Address(
                Address::ZERO,
            ))),
        }];
        let rows = vault_rows(&[Asset::native("eth".to_string())], &read, &[]).unwrap();
        assert!(matches!(rows[0].status, VaultStatus::Unavailable));
        assert!(rows[0].error.is_some());
    }

    fn holding(asset: AssetId, amount: u64, pending: bool) -> UnspentNote {
        let (symbol, token) = match &asset {
            AssetId::Native => ("eth".to_string(), None),
            AssetId::Erc20(address) => ("dai".to_string(), Some(*address)),
        };
        UnspentNote {
            asset,
            symbol,
            decimals: 18,
            token,
            amount_wei: U256::from(amount),
            pending,
            pool_id: "eth-0.1-1".to_string(),
        }
    }

    #[test]
    fn tornado_notes_join_a_shown_asset_and_skip_a_hidden_token() {
        let dai = Address::repeat_byte(0xaa);
        let assets = vec![Asset::native("eth".to_string())];
        let mut totals = vec![AssetTotals {
            public: Balance::spendable(U256::from(5_u64)),
            private: Balance::default(),
        }];
        fold_tornado(
            &assets,
            &mut totals,
            &[
                holding(AssetId::Native, 10, false),
                holding(AssetId::Native, 1, true),
                holding(AssetId::Erc20(dai), 100, false),
            ],
        );
        assert_eq!(totals[0].private.spendable, U256::from(10_u64));
        assert_eq!(totals[0].private.pending, U256::from(1_u64));
        assert_eq!(totals[0].public.spendable, U256::from(5_u64));
        assert_eq!(assets.len(), 1);

        let assets = vec![
            Asset::native("eth".to_string()),
            Asset {
                symbol: "DAI".to_string(),
                address: Some(dai.to_string()),
                decimals: 18,
                id: AssetId::Erc20(dai),
            },
        ];
        let mut totals = vec![AssetTotals::default(), AssetTotals::default()];
        fold_tornado(
            &assets,
            &mut totals,
            &[holding(AssetId::Erc20(dai), 100, false)],
        );
        assert_eq!(totals[1].private.spendable, U256::from(100_u64));
    }

    #[test]
    fn detailed_tornado_section_lists_each_note() {
        assert_eq!(tornado_lines(&[]), "\n\ntornado\n  no notes");
        let lines = tornado_lines(&[
            TornadoNoteRow {
                pool: "eth-0.1-1".to_string(),
                amount: "0.1 eth".to_string(),
                pending: false,
            },
            TornadoNoteRow {
                pool: "eth-1-1".to_string(),
                amount: "1 eth".to_string(),
                pending: true,
            },
        ]);
        assert_eq!(
            lines,
            "\n\ntornado\n  0.1 eth  eth-0.1-1\n  1 eth  eth-1-1  pending"
        );
    }

    #[test]
    fn a_vault_that_failed_to_build_is_unavailable_without_an_id() {
        let unbuilt = [UnbuiltVault {
            record: VaultRecord::new("no-such-vault"),
            error: FactoryError::NotFound("no-such-vault".to_string()),
        }];
        let rows = vault_rows(&[Asset::native("eth".to_string())], &[], &unbuilt).unwrap();
        assert!(matches!(rows[0].status, VaultStatus::Unavailable));
        assert_eq!(
            (rows[0].tag.as_str(), &rows[0].vault),
            ("no-such-vault", &None)
        );
    }
}
