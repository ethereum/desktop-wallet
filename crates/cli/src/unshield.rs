use std::fmt;

use alloy_primitives::{Address, U256};
use clap::{ArgGroup, Args};
use edw_core::vault::tornadocash::{deposit_pools, exact_notes, format_eth};
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    spend::{
        ShieldedWithdrawal, choose_profile, confirm, declined, ensure_paymaster, first_bundler,
        mnemonic, note_eth, open_vault, submit_shielded, tornado_amount,
    },
};

#[derive(Args, Debug)]
#[command(group = ArgGroup::new("amount").args(["amount_formatted", "amount_raw"]))]
pub struct UnshieldArgs {
    /// Decimal ETH to unshield, for example `1.1`. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_formatted: Option<String>,
    /// Wei to unshield. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_raw: Option<String>,
    /// Where the ETH is forwarded. Omitted, it stays on the fresh address.
    #[arg(long)]
    to: Option<String>,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Serialize)]
pub struct NoteLine {
    pub role: &'static str,
    pub amount: String,
}

#[derive(Serialize)]
struct UnshieldReport {
    fresh_index: u32,
    fresh_address: String,
    notes: Vec<NoteLine>,
    forward_to: Option<String>,
    forward_amount: Option<String>,
    fee: String,
    bundler: String,
    user_operation: String,
    transaction: String,
}

impl Report for UnshieldReport {
    const KIND: &'static str = "edw/unshield";
    const VERSION: u32 = 2;
}

impl fmt::Display for UnshieldReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "fresh {} {}", self.fresh_index, self.fresh_address)?;
        for note in &self.notes {
            writeln!(f, "{} {}", note.role, note.amount)?;
        }
        if let (Some(to), Some(amount)) = (&self.forward_to, &self.forward_amount) {
            writeln!(f, "forward {amount} ETH to {to}")?;
        }
        writeln!(f, "fee {} ETH", self.fee)?;
        writeln!(f, "bundler {}", self.bundler)?;
        writeln!(f, "user operation {}", self.user_operation)?;
        writeln!(f, "transaction {}", self.transaction)
    }
}

pub async fn run(global: &GlobalArgs, args: &UnshieldArgs) -> Result<(), anyhow::Error> {
    let instance = global.open().await?;
    let input = global.input();
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let amount = tornado_amount(
        args.amount_formatted.as_deref(),
        args.amount_raw.as_deref(),
        input,
    )?;
    let to = destination(args.to.as_deref(), input)?;
    let phrase = mnemonic(&instance, &profile).await?;
    let endpoint = instance.rpc().await?;
    let chain_id = endpoint.network_id().await?;
    deposit_pools(amount, chain_id)?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    eprintln!("Syncing Tornado pools...");
    let held = vault.notes().await?;
    let notes = exact_notes(amount, &held)?;
    ensure_paymaster(&notes)?;
    let fresh = instance
        .fresh_address(&profile, &phrase, endpoint.as_ref())
        .await?;
    let bundler = first_bundler(chain_id)?;
    if !confirm(
        input,
        &summary(amount, &notes, fresh.index, &fresh.address, to, &bundler),
    )? {
        declined();
        return Ok(());
    }
    let submitted = submit_shielded(ShieldedWithdrawal {
        phrase: &phrase,
        profile_index: profile.profile_index,
        endpoint: endpoint.as_ref(),
        vault: &vault,
        fresh_index: fresh.index,
        notes: &notes,
        forward_to: to,
        minimum_forward: None,
    })
    .await?;
    instance.commit_fresh_address(&profile, fresh.index).await?;
    let transaction = submitted.wait().await?;
    UnshieldReport {
        fresh_index: fresh.index,
        fresh_address: fresh.address.to_string(),
        notes: note_lines(&notes),
        forward_to: to.map(|address| address.to_string()),
        forward_amount: submitted.forwarded.map(format_eth),
        fee: format_eth(submitted.fee),
        bundler: submitted.bundler().unwrap_or(bundler),
        user_operation: submitted.hash.0.to_string(),
        transaction: transaction.to_string(),
    }
    .emit(global.mode())
}

fn destination(
    given: Option<&str>,
    input: crate::input::Input,
) -> Result<Option<Address>, anyhow::Error> {
    let text = match given {
        Some(value) => Some(value.to_string()),
        None => input.line("Forward to (empty leaves it on the fresh address): ")?,
    };
    match text.as_deref() {
        None | Some("") => Ok(None),
        Some(value) => Ok(Some(value.parse()?)),
    }
}

fn summary(
    amount: U256,
    notes: &[edw_core::vault::tornadocash::TornadoNote],
    index: u32,
    fresh: &Address,
    forward_to: Option<Address>,
    bundler: &str,
) -> String {
    let sponsor = notes.first().map(note_eth).unwrap_or_default();
    let mut lines = vec![format!(
        "Unshield {} ETH as {}.",
        format_eth(amount),
        notes
            .iter()
            .map(|note| format!("{} ETH", note_eth(note)))
            .collect::<Vec<_>>()
            .join(" + ")
    )];
    lines.push(format!(
        "Sponsoring note {sponsor} ETH pays gas through the Tornado paymaster."
    ));
    lines.push(format!("Runs as fresh address {index} ({fresh})."));
    lines.push(format!("Bundler {bundler}."));
    if let Some(to) = forward_to {
        lines.push(format!("Forwards the remainder, after that fee, to {to}."));
    } else {
        lines.push("The remainder stays on the fresh address.".to_string());
    }
    lines.join("\n")
}

pub fn note_lines(notes: &[edw_core::vault::tornadocash::TornadoNote]) -> Vec<NoteLine> {
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| NoteLine {
            role: if index == 0 { "sponsor" } else { "tail" },
            amount: format!("{} ETH", note_eth(note)),
        })
        .collect()
}
