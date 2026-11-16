use std::fmt;

use alloy_primitives::{Address, U256};
use clap::Args;
use edw_core::{
    instance::FreshAddress,
    vault::tornadocash::{Asset, TornadoNote, format_amount, withdrawal_notes},
};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    profile::ProfileArg,
    report::Report,
    spend::{
        AmountArgs, Consent, ShieldedWithdrawal, confirm, declined, ensure_paymaster,
        first_bundler, keep_fresh, mnemonic, open_vault, parse_tail_calls, quote_shielded,
        recipient_address, submit_shielded, tornado_asset,
    },
};

#[derive(Args, Debug)]
pub struct UnshieldArgs {
    #[command(flatten)]
    amount: AmountArgs,
    /// ETH, or a token with Tornado pools on this chain, by symbol or address. Omitted, it is the
    /// chain's only one, else picked at a terminal, else ETH.
    #[arg(long)]
    asset: Option<String>,
    /// Where the withdrawn asset is forwarded. Empty, or omitted without a terminal, it stays on
    /// the fresh address.
    #[arg(long)]
    to: Option<String>,
    /// Quote the sponsor fee, then stop without signing, sending, or storing the fresh address.
    #[arg(long)]
    dry_run: bool,
    /// Calls after withdrawal, from the fresh account: TARGET:CALLDATA[:WEI],...
    #[arg(long, hide_short_help = true, conflicts_with = "to", value_parser = validate_tail_calls)]
    tail_calls: Option<String>,
    /// Send ETH left after custom calls and the quoted fee to this address in the same `UserOp`.
    #[arg(
        long,
        hide_short_help = true,
        requires = "tail_calls",
        conflicts_with = "to"
    )]
    sweep_to: Option<String>,
    /// Abort if the next fresh account differs from this address.
    #[arg(long, hide_short_help = true)]
    expected_fresh_address: Option<Address>,
    #[command(flatten)]
    profile: ProfileArg,
    #[command(flatten)]
    consent: Consent,
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

#[derive(Serialize)]
struct UnshieldQuoteReport {
    fresh_index: u32,
    fresh_address: String,
    token: String,
    fee_token: String,
    fee_raw: String,
    available_raw: String,
    tail_value_raw: String,
    sweep_value_raw: Option<String>,
    bundler: String,
    tail_calls: usize,
    notes: Vec<NoteLine>,
}

impl Report for UnshieldQuoteReport {
    const KIND: &'static str = "edw/unshield-dry-run";
    const VERSION: u32 = 1;
}

impl fmt::Display for UnshieldQuoteReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Quote: fee {} wei; available {} base units; fresh {} {}. No transaction was sent.",
            self.fee_raw, self.available_raw, self.fresh_index, self.fresh_address
        )
    }
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
    let input = global.input();
    if !args.dry_run {
        args.consent.ensure_possible(input)?;
    }
    let instance = global.open().await?;
    let profile = args.profile.resolve(&instance, input).await?;
    let endpoint = instance.rpc().await?;
    let chain_id = endpoint.network_id().await?;
    let asset = tornado_asset(args.asset.as_deref(), chain_id, input)?;
    let tails = custom_calls(args, &asset)?;
    let amount = args
        .amount
        .resolve(input, &asset, chain_id, &instance, &profile)
        .await?;
    let to = if let Some(sweep) = &args.sweep_to {
        if asset != edw_core::vault::tornadocash::Asset::ETH {
            return Err(failure::usage(
                "--sweep-to currently supports ETH unshields only",
            ));
        }
        Some(recipient_address(sweep)?)
    } else if tails.is_empty() {
        destination(args.to.as_deref(), input)?
    } else {
        None
    };
    let phrase = mnemonic(&instance, &profile).await?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    eprintln!("Syncing Tornado pools...");
    let held = vault.notes().await?;
    let notes = withdrawal_notes(amount, &held, &asset, chain_id)?;
    ensure_paymaster(&notes)?;
    let fresh = instance
        .fresh_address(&profile, &phrase, endpoint.as_ref())
        .await?;
    verify_fresh(args.expected_fresh_address, fresh.address)?;
    let bundler_urls = instance.bundler_urls().await?;
    let bundler = first_bundler(&bundler_urls)?;
    let withdrawal = ShieldedWithdrawal {
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
    };
    if args.dry_run {
        return emit_quote(
            global,
            withdrawal,
            &fresh,
            &asset,
            &notes,
            bundler,
            tails.len(),
        )
        .await;
    }
    let description = confirmation(amount, &asset, &fresh, &notes, tails.len(), to, &bundler);
    for call in &tails {
        eprintln!("tail {}:{}:{}", call.target, call.data, call.value);
    }
    if !confirm(input, &args.consent, &description)? {
        declined();
        return Ok(());
    }
    let submitted = submit_shielded(withdrawal).await?;
    keep_fresh(&instance, &profile, &fresh).await;
    let transaction = submitted.wait().await?.transaction;
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

async fn emit_quote(
    global: &GlobalArgs,
    withdrawal: ShieldedWithdrawal<'_>,
    fresh: &FreshAddress,
    asset: &Asset,
    notes: &[TornadoNote],
    bundler: String,
    tail_calls: usize,
) -> Result<(), anyhow::Error> {
    let quoted = quote_shielded(withdrawal).await?;
    let sum: U256 = notes
        .iter()
        .filter(|note| note.pool.asset == *asset)
        .map(|note| U256::from(note.pool.amount_wei))
        .sum();
    let available = if asset == &Asset::ETH {
        sum.saturating_sub(quoted.fee)
    } else {
        sum
    };
    UnshieldQuoteReport {
        fresh_index: fresh.index,
        fresh_address: fresh.address.to_string(),
        token: asset.symbol().to_uppercase(),
        fee_token: "ETH".to_string(),
        fee_raw: quoted.fee.to_string(),
        available_raw: available.to_string(),
        tail_value_raw: if asset == &Asset::ETH {
            available
        } else {
            U256::ZERO
        }
        .to_string(),
        sweep_value_raw: quoted.forwarded.map(|value| value.to_string()),
        bundler,
        tail_calls,
        notes: note_lines(notes),
    }
    .emit(global.mode())
}

fn confirmation(
    amount: U256,
    asset: &Asset,
    fresh: &FreshAddress,
    notes: &[TornadoNote],
    tail_calls: usize,
    to: Option<Address>,
    bundler: &str,
) -> String {
    let description = format!(
        "Unshield {} {} to fresh {} ({}).\n{}\n{} custom tail calls. Bundler {bundler}.",
        format_amount(amount, asset.decimals()),
        asset.symbol().to_uppercase(),
        fresh.index,
        fresh.address,
        note_lines(notes)
            .iter()
            .map(|note| format!("{} {}", note.role, note.amount))
            .collect::<Vec<_>>()
            .join(" + "),
        tail_calls,
    );
    if let Some(to) = to {
        format!(
            "{description}\nForward remaining {} to {to}.",
            asset.symbol().to_uppercase()
        )
    } else {
        format!("{description}\nRemainder stays on the fresh account.")
    }
}

fn verify_fresh(expected: Option<Address>, actual: Address) -> Result<(), anyhow::Error> {
    if let Some(expected) = expected
        && expected != actual
    {
        return Err(failure::usage(format!(
            "expected fresh address {expected}, but the next account is {actual}; rebuild calls for this account"
        )));
    }
    Ok(())
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
    if let Some(value) = given {
        return if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(recipient_address(value)?))
        };
    }
    match input.ask(
        "--to",
        "Forward to (empty leaves it on the fresh address): ",
    )? {
        Some(value) if !value.is_empty() => Ok(Some(recipient_address(&value)?)),
        _ => Ok(None),
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn dry_run_and_sweep_parse_without_broadcast_consent() {
        let tail = "0x1111111111111111111111111111111111111111:0x:1";
        assert!(
            crate::Cli::try_parse_from([
                "edw",
                "unshield",
                "--dry-run",
                "--tail-calls",
                tail,
                "--non-interactive"
            ])
            .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "edw",
                "unshield",
                "--tail-calls",
                tail,
                "--sweep-to",
                "0x2222222222222222222222222222222222222222",
                "--yes"
            ])
            .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "edw",
                "unshield",
                "--sweep-to",
                "0x2222222222222222222222222222222222222222"
            ])
            .is_err()
        );
    }
    #[test]
    fn expected_address_is_optional_but_mismatch_aborts() {
        let actual = Address::repeat_byte(0x11);
        assert!(verify_fresh(None, actual).is_ok());
        assert!(verify_fresh(Some(actual), actual).is_ok());
        assert!(verify_fresh(Some(Address::repeat_byte(0x22)), actual).is_err());
    }
}
