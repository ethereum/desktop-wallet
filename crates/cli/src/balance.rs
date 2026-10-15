use std::fmt;

use alloy_primitives::U256;
use clap::Args;
use edw_core::{
    asset::{AssetId, NATIVE_DECIMALS, format_amount},
    balance::{self, AssetTotals, VaultBalances},
    factory::FactoryError,
    instance::Instance,
    profile::ProfileRecord,
    vault::{Balance, Vault, Visibility, db::VaultRecord},
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
    /// Also list each vault's balance.
    #[arg(long)]
    verbose: bool,
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
struct BalanceReport {
    profile: ProfileSummary,
    complete: bool,
    assets: Vec<AssetRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vaults: Option<Vec<VaultRow>>,
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

        let (vaults, unbuilt) = build_vaults(&instance, &profile, global).await?;
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

        BalanceReport {
            profile: ProfileSummary::from(&profile),
            complete: unbuilt.is_empty() && read.iter().all(|vault| vault.balances.is_ok()),
            assets: assets
                .iter()
                .zip(balance::totals(&read, assets.len()))
                .map(|(asset, totals)| asset.row(&totals))
                .collect::<anyhow::Result<_>>()?,
            vaults: self
                .verbose
                .then(|| vault_rows(&assets, &read, &unbuilt))
                .transpose()?,
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
    const VERSION: u32 = 1;
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
            write!(f, "\n\nTotals leave out the vaults that could not be read.")?;
        }

        let Some(vaults) = &self.vaults else {
            return Ok(());
        };
        if vaults.is_empty() {
            return write!(f, "\n\nNo vaults.");
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
        Ok(())
    }
}

/// Builds `profile`'s vaults, connecting to the endpoint only when there are any.
async fn build_vaults(
    instance: &Instance,
    profile: &ProfileRecord,
    global: &GlobalArgs,
) -> anyhow::Result<(Vec<Box<dyn Vault>>, Vec<UnbuiltVault>)> {
    let records = instance.vault_records(profile).await?;
    if records.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
    let built = instance.build_vaults(profile, &records, &endpoint).await;

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

    #[test]
    fn pending_shows_beside_spendable() {
        let balance = Asset::native("eth".to_string())
            .balance(Balance {
                spendable: U256::from(1_500_000_000_000_000_000_u128),
                pending: U256::from(100_000_000_000_000_000_u128),
            })
            .unwrap();
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
