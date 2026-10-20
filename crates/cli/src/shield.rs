use std::fmt;

use alloy_primitives::U256;
use clap::{ArgGroup, Args};
use edw_core::vault::tornadocash::{deposit_pools, format_eth};
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    spend::{
        account_at, choose_profile, confirm, declined, mnemonic, open_vault, send_one,
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
    transactions: Vec<String>,
}

impl Report for ShieldReport {
    const KIND: &'static str = "edw/shield";
    const VERSION: u32 = 1;
}

impl fmt::Display for ShieldReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "from {}", self.from)?;
        for note in &self.notes {
            writeln!(f, "note {note}")?;
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
            .map(|line| line.parse::<u32>())
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
    let summary = format!(
        "Shield {} ETH as {} from {from} ({}).\nGas is paid by that same account.",
        format_eth(amount),
        notes.join(" + "),
        payer.address
    );
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
    let mut transactions = Vec::with_capacity(pools.len());
    for (index, pool) in pools.iter().enumerate() {
        let calls = vault.deposit(pool).await?;
        for call in &calls {
            match send_one(endpoint.as_ref(), payer.signer.clone(), call).await {
                Ok(hash) => {
                    eprintln!(
                        "Sent {} ({}/{}): {hash}",
                        notes[index],
                        index + 1,
                        pools.len()
                    );
                    transactions.push(hash);
                }
                Err(error) => {
                    anyhow::bail!(
                        "{error}. Landed: {}. Not sent: {}",
                        if transactions.is_empty() {
                            "none".to_string()
                        } else {
                            transactions.join(", ")
                        },
                        notes[index..].join(", ")
                    );
                }
            }
        }
    }
    ShieldReport {
        from: payer.address.to_string(),
        notes,
        transactions,
    }
    .emit(global.mode())
}
