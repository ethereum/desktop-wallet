use std::fmt;

use alloy_primitives::{Address, U256};
use clap::{ArgGroup, Args};
use edw_core::{
    broadcast::submit_user_operation,
    bundler::Url,
    call::Call,
    network::NetworkEndpoint,
    vault::tornadocash::{deposit_pools, format_eth},
};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    report::Report,
    spend::{
        Account, account_at, choose_profile, confirm, declined, mnemonic, open_vault, send_one,
        tornado_amount,
    },
};

#[derive(Args, Debug)]
#[command(group = ArgGroup::new("amount").args(["amount_formatted", "amount_raw"]))]
pub struct ShieldArgs {
    /// Decimal ETH to shield, for example `1.1`. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_formatted: Option<String>,
    /// Wei to shield. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_raw: Option<String>,
    /// Standard-address index that pays. Defaults to 0.
    #[arg(long)]
    from: Option<u32>,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Serialize)]
struct ShieldReport {
    from: String,
    notes: Vec<String>,
    user_operation: Option<String>,
    bundler: Option<String>,
    transactions: Vec<String>,
}

impl Report for ShieldReport {
    const KIND: &'static str = "edw/shield";
    const VERSION: u32 = 2;
}

impl fmt::Display for ShieldReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "from {}", self.from)?;
        for note in &self.notes {
            writeln!(f, "note {note}")?;
        }
        if let Some(user_operation) = &self.user_operation {
            writeln!(f, "user operation {user_operation}")?;
        }
        if let Some(bundler) = &self.bundler {
            writeln!(f, "bundler {bundler}")?;
        }
        for transaction in &self.transactions {
            writeln!(f, "transaction {transaction}")?;
        }
        Ok(())
    }
}

pub async fn run(global: &GlobalArgs, args: &ShieldArgs) -> Result<(), anyhow::Error> {
    let instance = global.open().await?;
    let input = global.input();
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let amount = tornado_amount(
        args.amount_formatted.as_deref(),
        args.amount_raw.as_deref(),
        input,
    )?;
    let from = match args.from {
        Some(index) => index,
        None => input
            .line("Pay from address index [0]: ")?
            .filter(|line| !line.is_empty())
            .map(|line| {
                line.parse::<u32>()
                    .map_err(|_| failure::usage(format!("`{line}` is not an address index")))
            })
            .transpose()?
            .unwrap_or(0),
    };
    let phrase = mnemonic(&instance, &profile).await?;
    let payer = account_at(&phrase, profile.profile_index, from)?;
    let endpoint = instance.rpc().await?;
    let chain_id = endpoint.network_id().await?;
    let pools = deposit_pools(amount, chain_id)?;
    let notes: Vec<String> = pools
        .iter()
        .map(|pool| format!("{} ETH", format_eth(U256::from(pool.amount_wei))))
        .collect();
    let summary = confirmation(amount, &notes, from, payer.address, pools.len() > 1);
    if !confirm(input, &summary)? {
        declined();
        return Ok(());
    }
    let balance = endpoint.balance(payer.address).await?;
    let total: U256 = pools.iter().map(|pool| U256::from(pool.amount_wei)).sum();
    if balance <= total {
        anyhow::bail!(
            "{} has {} ETH, which does not cover {} ETH plus gas",
            payer.address,
            format_eth(balance),
            format_eth(total)
        );
    }
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    let mut calls = Vec::new();
    for pool in &pools {
        calls.extend(vault.deposit(pool).await?);
    }
    let bundler_urls = if calls.len() > 1 {
        instance.bundler_urls().await?
    } else {
        Vec::new()
    };
    let (transactions, user_operation, bundler) =
        broadcast(endpoint.as_ref(), &payer, &notes, &calls, &bundler_urls).await?;
    ShieldReport {
        from: payer.address.to_string(),
        notes,
        user_operation,
        bundler,
        transactions,
    }
    .emit(global.mode())
}

fn confirmation(
    amount: U256,
    notes: &[String],
    from: u32,
    payer: Address,
    batched: bool,
) -> String {
    let movement = format!(
        "Shield {} ETH as {} from {from} ({payer}).",
        format_eth(amount),
        notes.join(" + ")
    );
    if batched {
        format!(
            "{movement}\nOne user operation from that account. It installs the 7702 delegation when the account does not already have it. Gas is paid by that same account."
        )
    } else {
        format!("{movement}\nGas is paid by that same account.")
    }
}

async fn broadcast(
    endpoint: &dyn NetworkEndpoint,
    payer: &Account,
    notes: &[String],
    calls: &[Call],
    bundler_urls: &[Url],
) -> Result<(Vec<String>, Option<String>, Option<String>), anyhow::Error> {
    if calls.is_empty() {
        anyhow::bail!("shield produced no calls");
    }
    if calls.len() > 1 {
        return match submit_user_operation(endpoint, &payer.key, calls, bundler_urls).await {
            Ok(submitted) => {
                eprintln!(
                    "Submitted {} as one user operation {}",
                    notes.join(", "),
                    submitted.hash
                );
                Ok((
                    vec![submitted.transaction],
                    Some(submitted.hash),
                    submitted.bundler,
                ))
            }
            Err(error) => {
                anyhow::bail!("{error}. Recorded locally, not sent: {}", notes.join(", "))
            }
        };
    }
    let mut transactions = Vec::with_capacity(calls.len());
    for (index, call) in calls.iter().enumerate() {
        match send_one(endpoint, payer.signer.clone(), call).await {
            Ok(hash) => {
                let label = notes.get(index).map_or("note", String::as_str);
                eprintln!("Sent {label} ({}/{}): {hash}", index + 1, notes.len());
                transactions.push(hash);
            }
            Err(error) => {
                let landed = if transactions.is_empty() {
                    "none".to_string()
                } else {
                    transactions.join(", ")
                };
                let pending = notes.get(index..).unwrap_or_default().join(", ");
                anyhow::bail!("{error}. Landed: {landed}. Not sent: {pending}");
            }
        }
    }
    Ok((transactions, None, None))
}
