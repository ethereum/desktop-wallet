use std::fmt;

use alloy_primitives::{Address, U256};
use clap::{ArgGroup, Args};
use edw_core::{
    broadcast::submit_user_operation,
    bundler::Url,
    call::Call,
    instance::Instance,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    vault::tornadocash::{format_eth, parse_eth},
};
use futures::{StreamExt, TryStreamExt, stream};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    input::Input,
    report::Report,
    spend::{
        Account, Consent, account_at, choose_profile, confirm, declined, mnemonic, open_vault,
        send_one, tornado_amount,
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
    /// Exact pool sizes in ETH, separated by commas, for example `1,0.1,0.1`.
    #[arg(long)]
    denominations: Option<String>,
    /// Standard-address index that pays. Interactive mode offers funded indexes.
    #[arg(long)]
    from: Option<u32>,
    #[arg(long)]
    profile: Option<String>,
    #[command(flatten)]
    consent: Consent,
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
    let input = global.input();
    args.consent.ensure_possible(input)?;
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let requested_denominations = args
        .denominations
        .as_deref()
        .map(parse_denominations)
        .transpose()?;
    let requested_amount = if requested_denominations.is_none()
        || args.amount_formatted.is_some()
        || args.amount_raw.is_some()
    {
        Some(tornado_amount(
            args.amount_formatted.as_deref(),
            args.amount_raw.as_deref(),
            input,
        )?)
    } else {
        None
    };
    let phrase = mnemonic(&instance, &profile).await?;
    let endpoint = instance.rpc().await?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    let pools = if let Some(denominations) = &requested_denominations {
        vault.deposit_pools_for_denominations(denominations).await?
    } else {
        vault
            .deposit_pools(
                requested_amount.ok_or_else(|| anyhow::anyhow!("shield amount is required"))?,
            )
            .await?
    };
    let amount: U256 = pools.iter().map(|pool| U256::from(pool.amount_wei)).sum();
    verify_requested_amount(amount, requested_amount)?;
    let from = match args.from {
        Some(index) => index,
        None if input == Input::Terminal => {
            choose_funded_payer(&instance, &profile, endpoint.as_ref(), amount, input).await?
        }
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
    let payer = account_at(&phrase, profile.profile_index, from)?;
    let notes: Vec<String> = pools
        .iter()
        .map(|pool| {
            format!(
                "{} ETH{}",
                format_eth(U256::from(pool.amount_wei)),
                if pool.canonical { "" } else { " (extended)" }
            )
        })
        .collect();
    let summary = confirmation(amount, &notes, from, payer.address, pools.len() > 1);
    if !confirm(input, &args.consent, &summary)? {
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

async fn choose_funded_payer(
    instance: &Instance,
    profile: &ProfileRecord,
    endpoint: &dyn NetworkEndpoint,
    amount: U256,
    input: Input,
) -> Result<u32, anyhow::Error> {
    let addresses = instance.stored_address_accounts(profile).await?;
    let balances: Vec<(u32, U256)> = stream::iter(addresses)
        .map(|account| async move {
            Ok::<_, anyhow::Error>((account.index, endpoint.balance(account.address).await?))
        })
        .buffered(8)
        .try_collect()
        .await?;
    let funded = funded_indexes(amount, &balances);
    let Some(&default) = funded.first() else {
        anyhow::bail!(
            "no stored address has more than {} ETH for the shield; gas is also required. Fund an address or pass --from <index>",
            format_eth(amount)
        );
    };
    let listed = funded
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let prompt = format!("Pay from address index [{listed}] (default {default}): ");
    let entered = input.line(&prompt)?.unwrap_or_default();
    if entered.is_empty() {
        return Ok(default);
    }
    let index = entered
        .parse::<u32>()
        .map_err(|_| failure::usage(format!("`{entered}` is not an address index")))?;
    if !funded.contains(&index) {
        return Err(failure::usage(format!(
            "address index {index} is not among the funded choices [{listed}]"
        )));
    }
    Ok(index)
}

fn funded_indexes(amount: U256, balances: &[(u32, U256)]) -> Vec<u32> {
    let mut funded: Vec<u32> = balances
        .iter()
        .filter_map(|(index, balance)| (*balance > amount).then_some(*index))
        .collect();
    funded.sort_unstable();
    funded.dedup();
    funded
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
    let guidance = if notes.iter().any(|note| note.contains("(extended)")) {
        " Extended pools are newer and may have smaller anonymity sets. Use --denominations to choose exact pool sizes."
    } else {
        " Use --denominations to choose exact pool sizes."
    };
    if batched {
        format!(
            "{movement}{guidance}\nOne user operation from that account. It installs the 7702 delegation when the account does not already have it. Gas is paid by that same account."
        )
    } else {
        format!("{movement}{guidance}\nGas is paid by that same account.")
    }
}

fn parse_denominations(value: &str) -> Result<Vec<U256>, anyhow::Error> {
    if value.is_empty() {
        anyhow::bail!("--denominations needs one to five comma-separated ETH pool sizes");
    }
    value
        .split(',')
        .map(|part| {
            if part.trim().is_empty() {
                anyhow::bail!("--denominations contains an empty pool size");
            }
            Ok(parse_eth(part.trim())?)
        })
        .collect()
}

fn verify_requested_amount(actual: U256, requested: Option<U256>) -> Result<(), anyhow::Error> {
    if let Some(requested) = requested
        && actual != requested
    {
        anyhow::bail!(
            "--denominations sum to {} ETH, but the requested shield amount is {} ETH",
            format_eth(actual),
            format_eth(requested)
        );
    }
    Ok(())
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn explicit_denominations_parse_and_check_total() {
        let parsed = parse_denominations("1, 0.1,0.1").expect("denominations");
        let total: U256 = parsed.iter().copied().sum();
        assert_eq!(format_eth(total), "1.2");
        assert!(verify_requested_amount(total, Some(parse_eth("1.2").expect("amount"))).is_ok());
        assert!(verify_requested_amount(total, Some(parse_eth("1.1").expect("amount"))).is_err());
        assert!(parse_denominations("1,,0.1").is_err());
        assert!(
            crate::Cli::try_parse_from(["edw", "shield", "--denominations", "1,0.1", "--yes"])
                .is_ok()
        );
    }

    #[test]
    fn payer_choices_start_at_the_lowest_funded_index() {
        let amount = parse_eth("1").expect("amount");
        let headroom = parse_eth("0.1").expect("headroom");
        let balances = [
            (15, amount + headroom),
            (0, amount),
            (5, amount + headroom),
            (1, amount + headroom),
        ];
        assert_eq!(funded_indexes(amount, &balances), vec![1, 5, 15]);
        assert!(funded_indexes(amount, &[(0, amount)]).is_empty());
    }
}
