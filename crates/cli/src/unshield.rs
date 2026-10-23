use std::fmt;

use alloy_primitives::{Address, U256};
use clap::{ArgGroup, Args};
use edw_core::vault::tornadocash::format_eth;
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
    /// Pay gas with this standard-address index, allowing pools without a paymaster.
    #[arg(long)]
    gas_payer: Option<u32>,
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

#[derive(Serialize)]
struct PaidUnshieldReport {
    gas_payer: String,
    recipient: String,
    notes: Vec<String>,
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

impl Report for PaidUnshieldReport {
    const KIND: &'static str = "edw/unshield";
    const VERSION: u32 = 3;
}

impl fmt::Display for PaidUnshieldReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "gas payer {}", self.gas_payer)?;
        writeln!(f, "recipient {}", self.recipient)?;
        for note in &self.notes {
            writeln!(f, "note {note} ETH")?;
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
    let endpoint = instance.rpc().await?;
    let chain_id = endpoint.network_id().await?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    eprintln!("Syncing Tornado pools...");
    let mut notes = vault.exact_notes(amount).await?;
    if let Some(index) = args.gas_payer {
        return paid_withdrawal(
            global, &instance, &profile, &phrase, endpoint, &vault, &notes, to, index,
        )
        .await;
    }
    ensure_paymaster(&mut notes)?;
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

#[allow(clippy::too_many_arguments)]
async fn paid_withdrawal(
    global: &GlobalArgs,
    instance: &edw_core::instance::Instance,
    profile: &edw_core::profile::ProfileRecord,
    phrase: &edw_core::mnemonic::Mnemonic,
    endpoint: std::sync::Arc<dyn edw_core::network::NetworkEndpoint>,
    vault: &edw_core::vault::tornadocash::TornadoVault<alloy_signer_local::PrivateKeySigner>,
    notes: &[edw_core::vault::tornadocash::TornadoNote],
    to: Option<Address>,
    index: u32,
) -> anyhow::Result<()> {
    let payer = crate::spend::account_at(phrase, profile.profile_index, index)?;
    let fresh = if to.is_none() {
        Some(
            instance
                .fresh_address(profile, phrase, endpoint.as_ref())
                .await?,
        )
    } else {
        None
    };
    let recipient = to
        .or_else(|| fresh.as_ref().map(|fresh| fresh.address))
        .ok_or_else(|| anyhow::anyhow!("missing withdrawal recipient"))?;
    let amounts: Vec<String> = notes.iter().map(note_eth).collect();
    let summary = format!(
        "Unshield {} ETH to {recipient}. Standard account {index} ({}) pays gas; its transaction publicly links that account to these withdrawals.",
        amounts.join(" + "),
        payer.address
    );
    if !confirm(global.input(), &summary)? {
        declined();
        return Ok(());
    }
    let mut calls = Vec::new();
    for note in notes {
        calls.push(
            vault
                .withdraw(note, &edw_core::vault::VaultId::Address(recipient))
                .await?,
        );
    }
    if let Some(fresh) = fresh {
        instance.commit_fresh_address(profile, fresh.index).await?;
    }
    let transaction = if calls.len() == 1 {
        crate::spend::send_one(endpoint.as_ref(), payer.signer, &calls[0]).await?
    } else {
        edw_core::broadcast::submit_user_operation(endpoint.as_ref(), &payer.key, &calls)
            .await?
            .transaction
    };
    PaidUnshieldReport {
        gas_payer: payer.address.to_string(),
        recipient: recipient.to_string(),
        notes: amounts,
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
