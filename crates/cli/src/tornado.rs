use std::fmt;

use clap::{Args, Subcommand};
use edw_core::{profile::ProfileRecord, vault::tornadocash::Pool};
use serde::Serialize;

use crate::{
    GlobalArgs,
    profile::{ProfileSummary, choose_profile},
    report::Report,
};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Lists notes held by a profile in Tornado Cash pools.
    Notes(NotesArgs),
    /// Lists known Tornado Cash pools for the active network.
    Pools(PoolsArgs),
}

#[derive(Args, Debug)]
pub struct NotesArgs {
    /// Profile as `mnemonic/profile` or a unique name. Prompted for when more than one exists.
    selector: Option<String>,
    /// Track and check every known pool on the current network.
    #[arg(long)]
    all: bool,
    /// Scan historical events from pool deployment to recover any previously deposited notes.
    #[arg(long)]
    recover: bool,
}

#[derive(Args, Debug)]
pub struct PoolsArgs {}

#[derive(Serialize)]
pub struct NoteRow {
    pub pool: String,
    pub pool_address: String,
    pub nonce: u64,
    pub leaf_index: Option<u32>,
    pub spent: bool,
    pub status: String,
    pub amount: String,
}

#[derive(Serialize)]
pub struct NotesReport {
    pub profile: ProfileSummary,
    pub notes: Vec<NoteRow>,
}

#[derive(Serialize)]
pub struct PoolRow {
    pub pool: String,
    pub address: String,
    pub deployed_block: u64,
    pub amount: String,
}

#[derive(Serialize)]
pub struct PoolsReport {
    pub network: String,
    pub pools: Vec<PoolRow>,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::Notes(args) => args.run(global).await,
            Self::Pools(args) => args.run(global).await,
        }
    }
}

impl NotesArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let profile = if let Some(selector) = &self.selector {
            instance.profile(selector).await?
        } else {
            let mut profiles = instance.profiles().await?;
            profiles.sort_by_key(ProfileRecord::key);
            choose_profile(&profiles, global.input())?
        };

        let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
        let vault = instance.tornado_vault(&profile, endpoint).await?;

        let pools = if self.all || self.recover {
            let all = vault.all_pools().await?;
            vault.track(&all).await?;
            all
        } else {
            let tracked = vault.all_tracked_pools().await?;
            if tracked.is_empty() {
                let all = vault.all_pools().await?;
                vault.track(&all).await?;
                all
            } else {
                tracked
            }
        };

        if self.recover {
            vault.recover(&pools).await?;
        }

        let raw_notes = vault.notes().await?;
        let notes: Vec<NoteRow> = raw_notes
            .into_iter()
            .map(|note| {
                let status = if note.spent {
                    "spent".to_string()
                } else if note.leaf_index.is_some() {
                    "spendable".to_string()
                } else {
                    "pending".to_string()
                };
                let amount = format!(
                    "{} {}",
                    note.pool.amount(),
                    note.pool.symbol().to_uppercase()
                );
                NoteRow {
                    pool: note.pool.id(),
                    pool_address: note.pool.address.to_string(),
                    nonce: note.nonce,
                    leaf_index: note.leaf_index,
                    spent: note.spent,
                    status,
                    amount,
                }
            })
            .collect();

        NotesReport {
            profile: ProfileSummary::from(&profile),
            notes,
        }
        .emit(global.mode())
    }
}

impl PoolsArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
        let chain_id = endpoint.network_id().await?;
        let pools: Vec<PoolRow> = Pool::POOLS
            .iter()
            .filter(|p| p.chain_id == chain_id)
            .map(|p| PoolRow {
                pool: p.id(),
                address: p.address.to_string(),
                deployed_block: p.deployed_block,
                amount: format!("{} {}", p.amount(), p.symbol().to_uppercase()),
            })
            .collect();

        PoolsReport {
            network: chain_id.to_string(),
            pools,
        }
        .emit(global.mode())
    }
}

impl Report for NotesReport {
    const KIND: &'static str = "edw/tornado-notes";
    const VERSION: u32 = 1;
}

impl fmt::Display for NotesReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tornado Cash notes for {}", self.profile)?;
        if self.notes.is_empty() {
            return write!(f, "\nNo notes found.");
        }
        let rows: Vec<Vec<String>> = self
            .notes
            .iter()
            .map(|note| {
                vec![
                    note.pool.clone(),
                    note.nonce.to_string(),
                    note.leaf_index
                        .map_or_else(|| "pending".to_string(), |idx| idx.to_string()),
                    note.status.clone(),
                    note.amount.clone(),
                ]
            })
            .collect();
        write_table(f, "", &["pool", "nonce", "leaf", "status", "amount"], &rows)
    }
}

impl Report for PoolsReport {
    const KIND: &'static str = "edw/tornado-pools";
    const VERSION: u32 = 1;
}

impl fmt::Display for PoolsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tornado Cash pools for network {}", self.network)?;
        if self.pools.is_empty() {
            return write!(f, "\nNo pools configured for this network.");
        }
        let rows: Vec<Vec<String>> = self
            .pools
            .iter()
            .map(|p| {
                vec![
                    p.pool.clone(),
                    p.address.clone(),
                    p.deployed_block.to_string(),
                    p.amount.clone(),
                ]
            })
            .collect();
        write_table(
            f,
            "",
            &["pool", "address", "deployed_block", "amount"],
            &rows,
        )
    }
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
    use super::*;
    use crate::report::Mode;

    #[test]
    fn notes_report_json_format() {
        let profile = ProfileRecord::new(0, 0, Some("alice".to_string()));
        let report = NotesReport {
            profile: ProfileSummary::from(&profile),
            notes: vec![NoteRow {
                pool: "eth-0.1-1".to_string(),
                pool_address: "0x12D66f87A04A9E220743712cE6d9bB1B5616B8Fc".to_string(),
                nonce: 0,
                leaf_index: Some(42),
                spent: false,
                status: "spendable".to_string(),
                amount: "0.1 ETH".to_string(),
            }],
        };

        let json = report.format(Mode::Json).unwrap();
        assert!(json.contains(r#""kind":"edw/tornado-notes""#));
        assert!(json.contains(r#""status":"spendable""#));
        assert!(json.contains(r#""amount":"0.1 ETH""#));

        let human = report.format(Mode::Human).unwrap();
        assert!(human.contains("Tornado Cash notes for alice"));
        assert!(human.contains("eth-0.1-1"));
        assert!(human.contains("spendable"));
    }

    #[test]
    fn pools_report_json_format() {
        let report = PoolsReport {
            network: "1".to_string(),
            pools: vec![PoolRow {
                pool: "eth-0.1-1".to_string(),
                address: "0x12D66f87A04A9E220743712cE6d9bB1B5616B8Fc".to_string(),
                deployed_block: 9_116_966,
                amount: "0.1 ETH".to_string(),
            }],
        };

        let json = report.format(Mode::Json).unwrap();
        assert!(json.contains(r#""kind":"edw/tornado-pools""#));
        assert!(json.contains(r#""network":"1""#));

        let human = report.format(Mode::Human).unwrap();
        assert!(human.contains("Tornado Cash pools for network 1"));
        assert!(human.contains("9116966"));
    }
}
