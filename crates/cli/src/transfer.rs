use std::fmt;

use alloy_primitives::{Address, Bytes, U256};
use clap::Args;
use edw_core::{
    call::Call,
    vault::tornadocash::{cover_notes, format_eth},
};
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    spend::{
        ShieldedWithdrawal, account_at, account_by_address, choose_profile, confirm, declined,
        ensure_paymaster, eth_amount, first_bundler, mnemonic, note_eth, open_vault, send_one,
        submit_shielded,
    },
    unshield::note_lines,
};

#[derive(Args, Debug)]
pub struct TransferArgs {
    /// An index, a `0x` address of one, or `tornado`.
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Option<String>,
    #[arg(long)]
    amount: Option<String>,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Serialize)]
struct TransferReport {
    from: String,
    to: String,
    amount: String,
    fresh_index: Option<u32>,
    notes: Vec<crate::unshield::NoteLine>,
    change: Option<String>,
    forwarded: Option<String>,
    fee: Option<String>,
    bundler: Option<String>,
    user_operation: Option<String>,
    transaction: String,
}

impl Report for TransferReport {
    const KIND: &'static str = "edw/transfer";
    const VERSION: u32 = 2;
}

impl fmt::Display for TransferReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "from {}", self.from)?;
        writeln!(f, "to {}", self.to)?;
        writeln!(f, "amount {} ETH", self.amount)?;
        if let Some(index) = self.fresh_index {
            writeln!(f, "fresh {index}")?;
        }
        for note in &self.notes {
            writeln!(f, "{} {}", note.role, note.amount)?;
        }
        if let Some(change) = &self.change {
            writeln!(f, "change {change} ETH")?;
        }
        if let Some(forwarded) = &self.forwarded {
            writeln!(f, "forwarded {forwarded} ETH")?;
        }
        if let Some(fee) = &self.fee {
            writeln!(f, "fee {fee} ETH")?;
        }
        if let Some(bundler) = &self.bundler {
            writeln!(f, "bundler {bundler}")?;
        }
        if let Some(user_operation) = &self.user_operation {
            writeln!(f, "user operation {user_operation}")?;
        }
        writeln!(f, "transaction {}", self.transaction)
    }
}

pub async fn run(global: &GlobalArgs, args: &TransferArgs) -> Result<(), anyhow::Error> {
    let instance = global.open().await?;
    let input = global.input();
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let from = required(
        args.from.as_deref(),
        input,
        "From (index, address, or tornado): ",
    )?;
    let amount = eth_amount(args.amount.as_deref(), input, "Amount (ETH): ")?;
    let to: Address = required(args.to.as_deref(), input, "To: ")?.parse()?;
    let phrase = mnemonic(&instance, &profile).await?;
    let endpoint = instance.rpc().await?;
    if from.eq_ignore_ascii_case("tornado") {
        return tornado(
            global, &instance, input, &profile, &phrase, endpoint, amount, to,
        )
        .await;
    }
    let payer = if let Ok(index) = from.parse::<u32>() {
        account_at(&phrase, profile.profile_index, index)?
    } else {
        account_by_address(&phrase, profile.profile_index, from.parse()?)?
    };
    let summary = format!(
        "Transfer {} ETH from {} ({}) to {to}.\nGas is paid by that same account.",
        format_eth(amount),
        payer.index,
        payer.address
    );
    if !confirm(input, &summary)? {
        declined();
        return Ok(());
    }
    let balance = endpoint.balance(payer.address).await?;
    if balance <= amount {
        anyhow::bail!(
            "{} has {} ETH, which does not cover {} ETH plus gas",
            payer.address,
            format_eth(balance),
            format_eth(amount)
        );
    }
    let transaction = send_one(
        endpoint.as_ref(),
        payer.signer,
        &Call::new(to, Bytes::new(), amount),
    )
    .await?;
    TransferReport {
        from: payer.address.to_string(),
        to: to.to_string(),
        amount: format_eth(amount),
        fresh_index: None,
        notes: Vec::new(),
        change: None,
        forwarded: None,
        fee: None,
        bundler: None,
        user_operation: None,
        transaction,
    }
    .emit(global.mode())
}

#[allow(clippy::too_many_arguments)]
async fn tornado(
    global: &GlobalArgs,
    instance: &edw_core::instance::Instance,
    input: crate::input::Input,
    profile: &edw_core::profile::ProfileRecord,
    phrase: &edw_core::mnemonic::Mnemonic,
    endpoint: std::sync::Arc<dyn edw_core::network::NetworkEndpoint>,
    amount: U256,
    to: Address,
) -> Result<(), anyhow::Error> {
    let vault = open_vault(instance, profile, phrase, endpoint.clone()).await?;
    eprintln!("Syncing Tornado pools...");
    let held = vault.notes().await?;
    let notes = cover_notes(amount, &held)?;
    ensure_paymaster(&notes)?;
    let fresh = instance
        .fresh_address(profile, phrase, endpoint.as_ref())
        .await?;
    let bundler_urls = instance.bundler_urls().await?;
    let bundler = first_bundler(&bundler_urls)?;
    let sponsor = notes.first().map(note_eth).unwrap_or_default();
    let summary = format!(
        "Transfer {} ETH from tornado to {to}.\nUnshields {}.\nSponsoring note {sponsor} ETH pays gas through the Tornado paymaster.\nRuns as fresh address {} ({}).\nBundler {bundler}.\nForwards the remainder after that fee to {to}. The remainder has to cover {} ETH.",
        format_eth(amount),
        notes
            .iter()
            .map(|note| format!("{} ETH", note_eth(note)))
            .collect::<Vec<_>>()
            .join(" + "),
        fresh.index,
        fresh.address,
        format_eth(amount)
    );
    if !confirm(input, &summary)? {
        declined();
        return Ok(());
    }
    let submitted = submit_shielded(ShieldedWithdrawal {
        phrase,
        profile_index: profile.profile_index,
        endpoint: endpoint.as_ref(),
        vault: &vault,
        fresh_index: fresh.index,
        notes: &notes,
        forward_to: Some(to),
        minimum_forward: Some(amount),
        bundler_urls: &bundler_urls,
    })
    .await?;
    instance.commit_fresh_address(profile, fresh.index).await?;
    let transaction = submitted.wait().await?;
    TransferReport {
        from: "tornado".to_string(),
        to: to.to_string(),
        amount: format_eth(amount),
        fresh_index: Some(fresh.index),
        notes: note_lines(&notes),
        change: None,
        forwarded: submitted.forwarded.map(format_eth),
        fee: Some(format_eth(submitted.fee)),
        bundler: submitted.bundler().or(Some(bundler)),
        user_operation: Some(submitted.hash.0.to_string()),
        transaction: transaction.to_string(),
    }
    .emit(global.mode())
}

fn required(
    given: Option<&str>,
    input: crate::input::Input,
    label: &str,
) -> Result<String, anyhow::Error> {
    match given {
        Some(value) => Ok(value.to_string()),
        None => input
            .line(label)?
            .filter(|line| !line.is_empty())
            .ok_or_else(|| anyhow::anyhow!("pass {label}")),
    }
}
