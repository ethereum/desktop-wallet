use std::fmt;

use alloy_primitives::{Address, U256};
use clap::{ArgGroup, Args};
use edw_core::vault::tornadocash::{deposit_pools, exact_notes, format_eth};
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    spend::{
        account_at, choose_profile, confirm, declined, mnemonic, note_eth, open_vault,
        require_gas, send_batch, send_one, tornado_amount, withdrawal_calls,
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
    transaction: String,
}

impl Report for UnshieldReport {
    const KIND: &'static str = "edw/unshield";
    const VERSION: u32 = 1;
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
    let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
    let chain_id = endpoint.network_id().await?;
    deposit_pools(amount, chain_id)?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    eprintln!("Syncing Tornado pools...");
    let held = vault.notes().await?;
    let notes = exact_notes(amount, &held)?;
    let fresh = instance
        .fresh_address(&profile, &phrase, endpoint.as_ref())
        .await?;
    let context = account_at(&phrase, profile.profile_index, fresh.index)?;
    let payer = account_at(&phrase, profile.profile_index, 0)?;
    let withdrawn: U256 = notes
        .iter()
        .map(|note| U256::from(note.pool.amount_wei))
        .sum();
    let forward = to.map(|address| (address, withdrawn));
    if !confirm(
        input,
        &summary(amount, &notes, fresh.index, &context.address, forward),
    )? {
        declined();
        return Ok(());
    }
    require_gas(endpoint.as_ref(), payer.address).await?;
    let calls = withdrawal_calls(&vault, &notes, context.address, forward).await?;
    let transaction = if calls.len() == 1 {
        send_one(endpoint.as_ref(), payer.signer.clone(), &calls[0]).await?
    } else {
        send_batch(
            endpoint.as_ref(),
            context.signer.clone(),
            payer.signer.clone(),
            &calls,
        )
        .await?
    };
    instance.commit_fresh_address(&profile, fresh.index).await?;
    UnshieldReport {
        fresh_index: fresh.index,
        fresh_address: context.address.to_string(),
        notes: note_lines(&notes),
        forward_to: forward.map(|(address, _)| address.to_string()),
        forward_amount: forward.map(|(_, value)| format_eth(value)),
        transaction,
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
    forward: Option<(Address, U256)>,
) -> String {
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
        "Runs as fresh address {index} ({fresh}). Gas is paid by address 0."
    ));
    if let Some((to, value)) = forward {
        lines.push(format!("Forwards {} ETH to {to}.", format_eth(value)));
    } else {
        lines.push("The ETH stays on the fresh address.".to_string());
    }
    lines.join("\n")
}

pub fn note_lines(notes: &[edw_core::vault::tornadocash::TornadoNote]) -> Vec<NoteLine> {
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| NoteLine {
            role: if index == 0 { "main" } else { "tail" },
            amount: format!("{} ETH", note_eth(note)),
        })
        .collect()
}
