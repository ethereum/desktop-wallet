use std::fmt;

use alloy_primitives::{Address, U256};
use clap::{ArgGroup, Args};
use edw_core::vault::tornadocash::{
    deposit_pools_for_asset, format_amount, resolve_asset, withdrawal_notes,
};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    report::Report,
    spend::{
        ShieldedWithdrawal, choose_profile, confirm, declined, ensure_paymaster, first_bundler,
        mnemonic, open_vault, parse_address, parse_tail_calls, submit_shielded, token_amount,
    },
};

#[derive(Args, Debug)]
#[command(group = ArgGroup::new("amount").args(["amount_formatted", "amount_raw"]))]
pub struct UnshieldArgs {
    /// Decimal tokens to unshield, for example `1.1`. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_formatted: Option<String>,
    /// Base units to unshield. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_raw: Option<String>,
    /// Asset symbol: ETH (default), USDC, USDT, DAI, or WBTC.
    #[arg(long, default_value = "ETH")]
    token: String,
    /// Calls after withdrawal, from the fresh account: TARGET:CALLDATA[:WEI],...
    #[arg(long, conflicts_with = "to", value_parser = validate_tail_calls)]
    tail_calls: Option<String>,
    /// Where the selected asset is forwarded. Omitted, it stays on the fresh address.
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
    token: String,
    fee_token: String,
    tail_calls: usize,
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
            writeln!(f, "forward {amount} {} to {to}", self.token)?;
        }
        writeln!(f, "fee {} {}", self.fee, self.fee_token)?;
        writeln!(f, "bundler {}", self.bundler)?;
        writeln!(f, "user operation {}", self.user_operation)?;
        writeln!(f, "transaction {}", self.transaction)
    }
}

pub async fn run(global: &GlobalArgs, args: &UnshieldArgs) -> Result<(), anyhow::Error> {
    let instance = global.open().await?;
    let input = global.input();
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let endpoint = instance.rpc().await?;
    let chain_id = endpoint.network_id().await?;
    let asset = resolve_asset(&args.token, chain_id).map_err(failure::usage)?;
    let tails = custom_calls(args, &asset)?;
    let amount = token_amount(
        args.amount_formatted.as_deref(),
        args.amount_raw.as_deref(),
        input,
        &asset,
    )?;
    let to = if tails.is_empty() {
        destination(args.to.as_deref(), input)?
    } else {
        None
    };
    let phrase = mnemonic(&instance, &profile).await?;
    deposit_pools_for_asset(amount, chain_id, &asset)?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    eprintln!("Syncing Tornado pools...");
    let held = vault.notes().await?;
    let notes = withdrawal_notes(amount, &held, &asset, chain_id)?;
    ensure_paymaster(&notes)?;
    let fresh = instance
        .fresh_address(&profile, &phrase, endpoint.as_ref())
        .await?;
    let bundler_urls = instance.bundler_urls().await?;
    let bundler = first_bundler(&bundler_urls)?;
    let description = format!(
        "Unshield {} {} to fresh {} ({}).\n{}\n{} custom tail calls. Bundler {bundler}.",
        format_amount(amount, asset.decimals()),
        asset.symbol().to_uppercase(),
        fresh.index,
        fresh.address,
        note_lines(&notes)
            .iter()
            .map(|n| format!("{} {}", n.role, n.amount))
            .collect::<Vec<_>>()
            .join(" + "),
        tails.len()
    );
    let description = if let Some(to) = to {
        format!(
            "{description}\nForward remaining {} to {to}.",
            asset.symbol().to_uppercase()
        )
    } else {
        format!("{description}\nRemainder stays on the fresh account.")
    };
    for call in &tails {
        eprintln!("tail {}:{}:{}", call.target, call.data, call.value);
    }
    if !confirm(input, &description)? {
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
        tail_calls: &tails,
        asset: &asset,
        minimum_forward: None,
        bundler_urls: &bundler_urls,
    })
    .await?;
    instance.commit_fresh_address(&profile, fresh.index).await?;
    let transaction = submitted.wait().await?;
    UnshieldReport {
        fresh_index: fresh.index,
        fresh_address: fresh.address.to_string(),
        notes: note_lines(&notes),
        forward_to: to.map(|address| address.to_string()),
        forward_amount: submitted
            .forwarded
            .map(|value| format_amount(value, asset.decimals())),
        fee: format_amount(submitted.fee, notes[0].pool.asset.decimals()),
        token: asset.symbol().to_uppercase(),
        fee_token: notes[0].pool.symbol().to_uppercase(),
        tail_calls: tails.len(),
        bundler: submitted.bundler().unwrap_or(bundler),
        user_operation: submitted.hash.0.to_string(),
        transaction: transaction.to_string(),
    }
    .emit(global.mode())
}

fn custom_calls(
    args: &UnshieldArgs,
    asset: &edw_core::vault::tornadocash::Asset,
) -> Result<Vec<edw_core::call::Call>, anyhow::Error> {
    let tails = args
        .tail_calls
        .as_deref()
        .map(parse_tail_calls)
        .transpose()
        .map_err(failure::usage)?
        .unwrap_or_default();
    if edw_core::vault::tornadocash::token_address(asset).is_some()
        && tails.iter().any(|call| !call.value.is_zero())
    {
        return Err(failure::usage(
            "ERC-20 tail calls cannot include ETH msg.value",
        ));
    }
    Ok(tails)
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
        Some(value) => Ok(Some(parse_address(value)?)),
    }
}

pub fn note_lines(notes: &[edw_core::vault::tornadocash::TornadoNote]) -> Vec<NoteLine> {
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| NoteLine {
            role: if index == 0 { "sponsor" } else { "tail" },
            amount: format!(
                "{} {}",
                format_amount(U256::from(note.pool.amount_wei), note.pool.asset.decimals()),
                note.pool.symbol().to_uppercase()
            ),
        })
        .collect()
}

fn validate_tail_calls(raw: &str) -> Result<String, String> {
    parse_tail_calls(raw)?;
    Ok(raw.to_owned())
}
