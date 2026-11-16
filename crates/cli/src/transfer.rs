use std::{fmt, sync::Arc};

use alloy_primitives::{Bytes, U256};
use clap::Args;
use edw_core::{
    asset::{
        Erc20Asset, NATIVE_DECIMALS, erc20_balance, format_amount, parse_amount, simulate_transfer,
        transfer_call,
    },
    broadcast::{PreparedTransaction, prepare_call, submit_prepared},
    call::Call,
    instance::Instance,
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    recipient::{Recipient, RecipientError, ResolvedRecipient},
    vault::tornadocash::{TornadoNote, UserOpError, cover_notes, format_eth},
};
use serde::Serialize;

use crate::{
    GlobalArgs,
    asset::address_of,
    failure,
    input::{Choice, Input},
    profile::ProfileArg,
    report::Report,
    spend::{
        Account, Consent, ShieldedWithdrawal, account_at, account_by_address, confirm, declined,
        ensure_paymaster, first_bundler, keep_fresh, measured, mnemonic, note_eth, open_vault,
        quote_shielded,
    },
    unshield::{NoteLine, note_lines},
};

const NOTE_ATTEMPTS: usize = 3;

#[derive(Args, Debug)]
pub struct TransferArgs {
    /// An index, a `0x` address of one, or `tornado` to send exactly `--amount` of ETH from
    /// Tornado notes.
    #[arg(long)]
    from: Option<String>,
    /// An address, or an ENS name.
    #[arg(long)]
    to: Option<String>,
    /// In the asset's own units, for example `1.5`.
    #[arg(long)]
    amount: Option<String>,
    /// ETH, or a tracked ERC-20 by symbol or address. Picked at a terminal unless `--from` is
    /// `tornado`, else ETH.
    #[arg(long)]
    asset: Option<String>,
    #[command(flatten)]
    profile: ProfileArg,
    /// Print the transaction and its fee estimate, then stop without signing.
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    consent: Consent,
}

#[derive(Serialize)]
struct TransferReport {
    dry_run: bool,
    from: String,
    from_index: Option<u32>,
    to: String,
    to_name: Option<String>,
    amount: String,
    asset: String,
    token: Option<String>,
    unsigned: Option<UnsignedLines>,
    fresh_index: Option<u32>,
    fresh_address: Option<String>,
    notes: Vec<NoteLine>,
    fee: Option<String>,
    change: Option<String>,
    max_fee: Option<String>,
    bundler: Option<String>,
    user_operation: Option<String>,
    transaction: Option<String>,
}

#[derive(Serialize)]
struct UnsignedLines {
    target: String,
    value: String,
    data: String,
    nonce: u64,
    gas_limit: u64,
    max_fee_per_gas_gwei: String,
    max_priority_fee_per_gas_gwei: String,
}

enum Asset {
    Eth,
    Erc20(Erc20Asset),
}

struct Plan<'a> {
    global: &'a GlobalArgs,
    args: &'a TransferArgs,
    instance: &'a Instance,
    input: Input,
    profile: &'a ProfileRecord,
    phrase: &'a Mnemonic,
    endpoint: Arc<dyn NetworkEndpoint>,
    asset: Asset,
    amount: U256,
    to: ResolvedRecipient,
}

impl Report for TransferReport {
    const KIND: &'static str = "edw/transfer";
    const VERSION: u32 = 4;
}

impl fmt::Display for TransferReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dry_run {
            writeln!(f, "dry run: nothing was signed or sent")?;
        }
        match self.from_index {
            Some(index) => writeln!(f, "from {index} {}", self.from)?,
            None => writeln!(f, "from {}", self.from)?,
        }
        match &self.to_name {
            Some(name) => writeln!(f, "to {name} {}", self.to)?,
            None => writeln!(f, "to {}", self.to)?,
        }
        writeln!(f, "amount {} {}", self.amount, self.asset)?;
        if let Some(token) = &self.token {
            writeln!(f, "token {token}")?;
        }
        if let Some(unsigned) = &self.unsigned {
            writeln!(f, "target {}", unsigned.target)?;
            writeln!(f, "value {} ETH", unsigned.value)?;
            writeln!(f, "data {}", unsigned.data)?;
            writeln!(f, "nonce {}", unsigned.nonce)?;
            writeln!(f, "gas limit {}", unsigned.gas_limit)?;
            writeln!(f, "max fee per gas {} gwei", unsigned.max_fee_per_gas_gwei)?;
            writeln!(
                f,
                "max priority fee per gas {} gwei",
                unsigned.max_priority_fee_per_gas_gwei
            )?;
        }
        if let (Some(index), Some(address)) = (self.fresh_index, &self.fresh_address) {
            writeln!(f, "fresh {index} {address}")?;
        }
        for note in &self.notes {
            writeln!(f, "{} {}", note.role, note.amount)?;
        }
        if let Some(fee) = &self.fee {
            writeln!(f, "fee {fee} ETH")?;
        } else if let Some(max_fee) = &self.max_fee {
            writeln!(f, "max fee {max_fee} ETH")?;
        }
        if let Some(change) = &self.change {
            let bound = if self.fee.is_none() { " at least" } else { "" };
            writeln!(f, "change{bound} {change} ETH")?;
        }
        if let Some(bundler) = &self.bundler {
            writeln!(f, "bundler {bundler}")?;
        }
        if let Some(user_operation) = &self.user_operation {
            writeln!(f, "user operation {user_operation}")?;
        }
        if let Some(transaction) = &self.transaction {
            writeln!(f, "transaction {transaction}")?;
        }
        Ok(())
    }
}

impl Asset {
    fn symbol(&self) -> &str {
        match self {
            Self::Eth => "ETH",
            Self::Erc20(asset) => &asset.symbol,
        }
    }

    const fn decimals(&self) -> u8 {
        match self {
            Self::Eth => NATIVE_DECIMALS,
            Self::Erc20(asset) => asset.decimals,
        }
    }

    fn format(&self, amount: U256) -> String {
        format_amount(amount, self.decimals()).unwrap_or_else(|_| amount.to_string())
    }
}

impl Plan<'_> {
    fn report(&self, from: String, from_index: Option<u32>) -> TransferReport {
        TransferReport {
            dry_run: self.args.dry_run,
            from,
            from_index,
            to: self.to.address.to_string(),
            to_name: self.to.name.clone(),
            amount: self.asset.format(self.amount),
            asset: self.asset.symbol().to_string(),
            token: match &self.asset {
                Asset::Eth => None,
                Asset::Erc20(asset) => Some(asset.address.to_string()),
            },
            unsigned: None,
            fresh_index: None,
            fresh_address: None,
            notes: Vec::new(),
            fee: None,
            change: None,
            max_fee: None,
            bundler: None,
            user_operation: None,
            transaction: None,
        }
    }
}

pub async fn run(global: &GlobalArgs, args: &TransferArgs) -> Result<(), anyhow::Error> {
    let input = global.input();
    if !args.dry_run {
        args.consent.ensure_possible(input)?;
    }
    let instance = global.open().await?;
    let profile = args.profile.resolve(&instance, input).await?;
    let from = required(
        args.from.as_deref(),
        input,
        "--from",
        "From (index, address, or tornado): ",
    )?;
    let tornado = from.eq_ignore_ascii_case("tornado");
    let asset = chosen_asset(&instance, &profile, args.asset.as_deref(), input, tornado).await?;
    if tornado && matches!(asset, Asset::Erc20(_)) {
        return Err(failure::usage(
            "Tornado holds only ETH; send an ERC-20 from an account index or address",
        ));
    }
    let amount = required(
        args.amount.as_deref(),
        input,
        "--amount",
        &format!("Amount ({}): ", asset.symbol()),
    )?;
    let amount = parse_amount(&amount, asset.decimals())?;
    if amount.is_zero() {
        return Err(failure::usage("amount must be greater than zero"));
    }
    let to: Recipient = required(
        args.to.as_deref(),
        input,
        "--to",
        "To (address or ENS name): ",
    )?
    .parse()
    .map_err(|error: RecipientError| failure::usage(error.to_string()))?;
    let phrase = mnemonic(&instance, &profile).await?;
    let endpoint = instance.rpc().await?;
    let to = to.resolve(endpoint.as_ref()).await?;
    let plan = Plan {
        global,
        args,
        instance: &instance,
        input,
        profile: &profile,
        phrase: &phrase,
        endpoint,
        asset,
        amount,
        to,
    };
    if tornado {
        return from_tornado(&plan).await;
    }
    let payer = if let Ok(index) = from.parse::<u32>() {
        account_at(&phrase, profile.profile_index, index)?
    } else {
        account_by_address(&phrase, profile.profile_index, &from)?
    };
    from_account(&plan, payer).await
}

async fn chosen_asset(
    instance: &Instance,
    profile: &ProfileRecord,
    selector: Option<&str>,
    input: Input,
    tornado: bool,
) -> Result<Asset, anyhow::Error> {
    let Some(selector) = selector else {
        if tornado || input != Input::Terminal {
            return Ok(Asset::Eth);
        }
        return pick_asset(&instance.profile_erc20s(profile).await?, input);
    };
    if selector.eq_ignore_ascii_case("eth") {
        return Ok(Asset::Eth);
    }
    named_asset(selector, &instance.profile_erc20s(profile).await?)
}

fn named_asset(selector: &str, tracked: &[Erc20Asset]) -> Result<Asset, anyhow::Error> {
    if selector.eq_ignore_ascii_case("eth") {
        return Ok(Asset::Eth);
    }
    let address = address_of(selector, tracked)?;
    tracked
        .iter()
        .find(|asset| Some(asset.address) == address)
        .map(|asset| Asset::Erc20(asset.clone()))
        .ok_or_else(|| {
            failure::usage(format!(
                "{selector} is not an ERC-20 this profile tracks; see `edw asset list`, or track it with `edw asset add <address>`"
            ))
        })
}

/// The `--asset` value that names `asset` again: its symbol, unless that names something else.
fn asset_selector(asset: &Erc20Asset, tracked: &[Erc20Asset]) -> String {
    match named_asset(&asset.symbol, tracked) {
        Ok(Asset::Erc20(named)) if named.address == asset.address => asset.symbol.clone(),
        _ => asset.address.to_string(),
    }
}

fn pick_asset(tracked: &[Erc20Asset], input: Input) -> Result<Asset, anyhow::Error> {
    if tracked.is_empty() {
        input.imply("--asset", Some("ETH".to_string()));
        return Ok(Asset::Eth);
    }
    let choices: Vec<Choice> = std::iter::once(Choice::new("ETH", "--asset", "ETH"))
        .chain(tracked.iter().map(|asset| {
            let selector = asset_selector(asset, tracked);
            let label = if selector == asset.symbol {
                selector.clone()
            } else {
                format!("{} ({})", asset.symbol, asset.address)
            };
            Choice::new(label, "--asset", selector)
        }))
        .collect();
    let Some(choice) = input.choose_or("Select an asset:", &choices, 0)? else {
        return Ok(Asset::Eth);
    };
    Ok(match choice.checked_sub(1) {
        Some(index) => Asset::Erc20(tracked[index].clone()),
        None => Asset::Eth,
    })
}

async fn from_account(plan: &Plan<'_>, payer: Account) -> Result<(), anyhow::Error> {
    let endpoint = plan.endpoint.as_ref();
    let call = match &plan.asset {
        Asset::Eth => Call::new(plan.to.address, Bytes::new(), plan.amount),
        Asset::Erc20(asset) => {
            let held = erc20_balance(endpoint, asset.address, payer.address).await?;
            if held < plan.amount {
                anyhow::bail!(
                    "{} holds {} {}, which does not cover {} {}; add {} {}",
                    payer.address,
                    plan.asset.format(held),
                    asset.symbol,
                    plan.asset.format(plan.amount),
                    asset.symbol,
                    plan.asset.format(plan.amount - held),
                    asset.symbol
                );
            }
            let transfer = transfer_call(asset.address, plan.to.address, plan.amount);
            simulate_transfer(endpoint, payer.address, &transfer).await?;
            transfer
        }
    };
    let balance = endpoint.balance(payer.address).await?;
    if balance < call.value {
        anyhow::bail!(
            "{} holds {} ETH, which does not cover {} ETH; add {} ETH, plus gas",
            payer.address,
            format_eth(balance),
            format_eth(call.value),
            format_eth(call.value - balance)
        );
    }
    let prepared = prepare_call(endpoint, payer.address, &call).await?;
    let needed = call.value + prepared.max_fee();
    if balance < needed {
        anyhow::bail!(
            "{} holds {} ETH, but this transfer needs up to {} ETH: {} ETH sent plus up to {} ETH of gas; add {} ETH",
            payer.address,
            format_eth(balance),
            format_eth(needed),
            format_eth(call.value),
            format_eth(prepared.max_fee()),
            format_eth(needed - balance)
        );
    }

    let mut report = plan.report(payer.address.to_string(), Some(payer.index));
    if plan.args.dry_run {
        report.unsigned = Some(unsigned(&call, &prepared));
        report.max_fee = Some(format_eth(prepared.max_fee()));
        return report.emit(plan.global.mode());
    }
    let summary = format!(
        "Transfer {} {} from {} ({}) to {}.\nGas, up to {} ETH, is paid by that same account.",
        plan.asset.format(plan.amount),
        plan.asset.symbol(),
        payer.index,
        payer.address,
        plan.to,
        format_eth(prepared.max_fee())
    );
    if !confirm(plan.input, &plan.args.consent, &summary)? {
        declined();
        return Ok(());
    }
    let included = submit_prepared(endpoint, payer.signer, prepared).await?;
    report.fee = Some(format_eth(included.fee));
    report.transaction = Some(included.hash.to_string());
    report.emit(plan.global.mode())
}

async fn from_tornado(plan: &Plan<'_>) -> Result<(), anyhow::Error> {
    let vault = open_vault(
        plan.instance,
        plan.profile,
        plan.phrase,
        plan.endpoint.clone(),
    )
    .await?;
    eprintln!("Syncing Tornado pools...");
    let held = vault.notes().await?;
    let fresh = plan
        .instance
        .fresh_address(plan.profile, plan.phrase, plan.endpoint.as_ref())
        .await?;
    let bundler_urls = plan.instance.bundler_urls().await?;
    let bundler = first_bundler(&bundler_urls)?;
    let send = [Call::new(plan.to.address, Bytes::new(), plan.amount)];
    let mut notes = cover_notes(plan.amount, &held)?;
    let mut attempts = 0;
    let quoted = loop {
        ensure_paymaster(&notes)?;
        let quoted = quote_shielded(ShieldedWithdrawal {
            phrase: plan.phrase,
            profile_index: plan.profile.profile_index,
            endpoint: plan.endpoint.as_ref(),
            vault: &vault,
            fresh_index: fresh.index,
            notes: &notes,
            asset: &edw_core::vault::tornadocash::Asset::ETH,
            forward_to: None,
            minimum_forward: None,
            tail_calls: &send,
            bundler_urls: &bundler_urls,
        })
        .await;
        attempts += 1;
        match quoted.map_err(anyhow::Error::downcast::<UserOpError>) {
            Ok(quoted) => break quoted,
            Err(Ok(UserOpError::Short { fee, .. })) if attempts < NOTE_ATTEMPTS => {
                eprintln!(
                    "The quoted fee leaves the notes short; selecting the smallest sufficient note sum."
                );
                let required = plan.amount.checked_add(fee).ok_or_else(|| {
                    anyhow::anyhow!("transfer amount plus paymaster fee overflows")
                })?;
                notes = cover_notes(required, &held)?;
            }
            Err(Ok(error)) => return Err(error.into()),
            Err(Err(error)) => return Err(error),
        }
    };
    let change = quoted.sum - quoted.fee - plan.amount;
    let mut report = plan.report("tornado".to_string(), None);
    report.fresh_index = Some(fresh.index);
    report.fresh_address = Some(fresh.address.to_string());
    report.notes = note_lines(&notes);
    report.max_fee = Some(format_eth(quoted.fee));
    report.change = Some(format_eth(change));
    if plan.args.dry_run {
        return report.emit(plan.global.mode());
    }
    let summary = format!(
        "Transfer {} ETH from tornado to {}.\nWithdraws {} to fresh address {} ({}).\nThe sponsoring {} ETH note pays a fee of up to {} ETH through the Tornado paymaster.\nAt least {} ETH of change stays on the fresh address.\nBundler {}.",
        format_eth(plan.amount),
        plan.to,
        note_amounts(&notes),
        fresh.index,
        fresh.address,
        notes.first().map(note_eth).unwrap_or_default(),
        format_eth(quoted.fee),
        format_eth(change),
        bundler,
    );
    if !confirm(plan.input, &plan.args.consent, &summary)? {
        declined();
        return Ok(());
    }
    keep_fresh(plan.instance, plan.profile, &fresh).await;
    let submitted = quoted.submit().await?;
    let included = submitted.wait().await?;
    if let Some(cost) = measured(&included).await {
        report.fee = Some(format_eth(cost.fee));
        report.change = Some(format_eth(cost.change));
    }
    report.bundler = submitted.bundler().or(Some(bundler));
    report.user_operation = Some(submitted.hash.0.to_string());
    report.transaction = Some(included.transaction.to_string());
    report.emit(plan.global.mode())
}

fn note_amounts(notes: &[TornadoNote]) -> String {
    notes
        .iter()
        .map(|note| format!("{} ETH", note_eth(note)))
        .collect::<Vec<_>>()
        .join(" + ")
}

fn unsigned(call: &Call, prepared: &PreparedTransaction) -> UnsignedLines {
    let gwei = |wei: u128| format_amount(U256::from(wei), 9).unwrap_or_else(|_| wei.to_string());
    UnsignedLines {
        target: call.target.to_string(),
        value: format_eth(call.value),
        data: call.data.to_string(),
        nonce: prepared.request.nonce.unwrap_or_default(),
        gas_limit: prepared.gas_limit(),
        max_fee_per_gas_gwei: gwei(prepared.max_fee_per_gas()),
        max_priority_fee_per_gas_gwei: gwei(
            prepared
                .request
                .max_priority_fee_per_gas
                .unwrap_or_default(),
        ),
    }
}

fn required(
    given: Option<&str>,
    input: Input,
    flag: &'static str,
    label: &str,
) -> Result<String, anyhow::Error> {
    if let Some(value) = given {
        return Ok(value.to_string());
    }
    input
        .ask(flag, label)?
        .filter(|line| !line.is_empty())
        .ok_or_else(|| failure::usage(format!("pass {flag}")))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use super::*;

    fn token(byte: u8, symbol: &str) -> Erc20Asset {
        Erc20Asset {
            address: Address::repeat_byte(byte),
            symbol: symbol.to_string(),
            decimals: 18,
        }
    }

    #[test]
    fn a_picked_token_is_named_again_by_its_recorded_selector() {
        let shaped = Address::repeat_byte(9).to_string();
        let tracked = [
            token(1, "DAI"),
            token(2, "eth"),
            token(3, "FOO"),
            token(4, "FOO"),
            token(5, &shaped),
        ];
        for asset in &tracked {
            let Ok(Asset::Erc20(named)) = named_asset(&asset_selector(asset, &tracked), &tracked)
            else {
                panic!("{} names no tracked token", asset.symbol)
            };
            assert_eq!(named.address, asset.address);
        }
    }
}
