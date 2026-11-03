use std::{
    fmt::{self, Write},
    sync::Arc,
};

use alloy_primitives::{Address, U256};
use clap::{Args, Subcommand, ValueEnum};
use edw_core::{
    asset::{AssetId, Erc20Asset, format_amount, parse_amount, transfer_call},
    call::Call,
    instance::Instance,
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    recipient::Recipient,
    vault::railgun::{
        Movement, RailgunNote, RailgunQuote, RailgunSession, RailgunVault, bundler::RailgunBundlers,
    },
};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    input::Input,
    report::Report,
    spend::{
        Consent, account_at, choose_profile, confirm, declined, keep_fresh, mnemonic,
        parse_tail_calls, recipient_address, send_one,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Protocol {
    Tornado,
    Railgun,
}

#[derive(Subcommand)]
pub enum Command {
    /// Prints the profile's Railgun receiving address without synchronizing.
    Address(ProfileArgs),
    /// Syncs notes and PPOI lists, recovers missing proofs and retries proof submission.
    Sync(SyncArgs),
    /// Resubmits the exact encrypted pending `UserOperation` after an interrupted send.
    Retry(RetryArgs),
}
#[derive(Args)]
pub struct ProfileArgs {
    #[arg(long)]
    profile: Option<String>,
}
#[derive(Args)]
pub struct SyncArgs {
    #[arg(long)]
    profile: Option<String>,
    /// Recover proofs locally without submitting them to a PPOI node.
    #[arg(long)]
    observe_only: bool,
}
#[derive(Args)]
pub struct RetryArgs {
    #[arg(long)]
    profile: Option<String>,
    #[command(flatten)]
    consent: Consent,
}

struct Asset {
    id: AssetId,
    token: Address,
    symbol: String,
    decimals: u8,
}

#[derive(Serialize)]
struct PublicCall {
    target: String,
    data: String,
    value: String,
}
#[derive(Serialize)]
struct RailgunReport {
    action: &'static str,
    address: String,
    token: Option<String>,
    amount_raw: Option<String>,
    fee_token: &'static str,
    fee_raw: Option<String>,
    protocol_fee_raw: Option<String>,
    quote: bool,
    fresh_index: Option<u32>,
    fresh_address: Option<String>,
    recipient: Option<String>,
    calls: Vec<PublicCall>,
    transactions: Vec<String>,
    bundler: Option<String>,
    user_operation: Option<String>,
    notes: Vec<RailgunNote>,
    poi: Option<serde_json::Value>,
    pending_user_operation: Option<String>,
}
impl RailgunReport {
    fn new(action: &'static str, vault: &RailgunVault) -> Self {
        Self {
            action,
            address: vault.address().to_string(),
            token: None,
            amount_raw: None,
            fee_token: "WETH",
            fee_raw: None,
            protocol_fee_raw: None,
            quote: false,
            fresh_index: None,
            fresh_address: None,
            recipient: None,
            calls: Vec::new(),
            transactions: Vec::new(),
            bundler: None,
            user_operation: None,
            notes: Vec::new(),
            poi: None,
            pending_user_operation: None,
        }
    }
}
impl Report for RailgunReport {
    const KIND: &'static str = "edw/railgun";
    const VERSION: u32 = 1;
}
impl fmt::Display for RailgunReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "railgun {}", self.address)?;
        if self.quote {
            writeln!(
                f,
                "Quote only; no spending signature or transaction was sent."
            )?;
        }
        if let (Some(amount), Some(token)) = (&self.amount_raw, &self.token) {
            writeln!(f, "{} {amount} base units of {token}", self.action)?;
        }
        if let Some(to) = &self.recipient {
            writeln!(f, "to {to}")?;
        }
        if let Some(address) = &self.fresh_address {
            writeln!(
                f,
                "fresh {} {address}",
                self.fresh_index.unwrap_or_default()
            )?;
        }
        if let Some(fee) = &self.fee_raw {
            writeln!(f, "maximum fee {fee} wei WETH (paid from private WETH)")?;
        }
        if let Some(fee) = &self.protocol_fee_raw {
            writeln!(f, "protocol fee {fee} base units")?;
        }
        for call in &self.calls {
            writeln!(f, "call {}:{}:{}", call.target, call.data, call.value)?;
        }
        for note in &self.notes {
            writeln!(
                f,
                "{} {}  PPOI {}  tree {} leaf {}",
                note.amount, note.token, note.poi_status, note.tree, note.leaf
            )?;
        }
        if let Some(poi) = &self.poi {
            writeln!(f, "PPOI {poi}")?;
        }
        if let Some(hash) = &self.pending_user_operation {
            writeln!(f, "pending {hash}; resume with `edw railgun retry`")?;
        }
        if let Some(bundler) = &self.bundler {
            writeln!(f, "bundler {bundler}")?;
        }
        if let Some(hash) = &self.user_operation {
            writeln!(f, "user operation {hash}")?;
        }
        for tx in &self.transactions {
            writeln!(f, "transaction {tx}")?;
        }
        Ok(())
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> anyhow::Result<()> {
        let selector = match self {
            Self::Address(a) => a.profile.as_deref(),
            Self::Sync(a) => a.profile.as_deref(),
            Self::Retry(a) => a.profile.as_deref(),
        };
        if let Self::Retry(args) = self {
            args.consent.ensure_possible(global.input())?;
        }
        let instance = global.open().await?;
        let profile = choose_profile(&instance, selector, global.input()).await?;
        let phrase = mnemonic(&instance, &profile).await?;
        let endpoint = instance.rpc().await?;
        let vault = open_vault(global, &instance, &profile, &phrase, endpoint).await?;
        let mut report = RailgunReport::new("address", &vault);
        if matches!(self, Self::Address(_)) {
            return report.emit(global.mode());
        }
        let mut session = vault.session().await?;
        match self {
            Self::Sync(args) => {
                report.action = "sync";
                report.poi = sync(&mut session, !args.observe_only).await?;
                report.notes = session.notes().await;
                report.pending_user_operation = session
                    .pending()
                    .map(|p| p.signed.hash(vault.chain().id).0.to_string());
            }
            Self::Retry(args) => {
                let pending = session
                    .pending()
                    .ok_or_else(|| failure::usage("no pending Railgun operation"))?
                    .clone();
                let hash = pending.signed.hash(vault.chain().id).0.to_string();
                if !confirm(
                    global.input(),
                    &args.consent,
                    &format!(
                        "Retry the exact pending Railgun operation {hash}, with maximum private WETH fee {} wei.",
                        pending.fee
                    ),
                )? {
                    declined();
                    return Ok(());
                }
                let address =
                    phrase.standard_address(pending.fresh_index, profile.profile_index)?;
                if address != pending.signed.user_op.sender {
                    return Err(failure::usage("pending sender does not match this profile"));
                }
                keep_fresh(
                    &instance,
                    &profile,
                    &edw_core::instance::FreshAddress {
                        index: pending.fresh_index,
                        address,
                    },
                )
                .await;
                let bundlers =
                    RailgunBundlers::from_urls(vault.chain().id, &instance.bundler_urls().await?)?;
                let (hash, tx) = session.submit_pending(&bundlers).await?;
                report.action = "retry";
                report.fee_raw = Some(pending.fee.to_string());
                report.user_operation = Some(hash);
                report.transactions.push(tx);
                report.bundler = bundlers.accepted_url();
                report.poi = sync(&mut session, true).await.unwrap_or_else(|e| {
                    eprintln!("Operation succeeded; later sync failed: {e:#}");
                    None
                });
            }
            Self::Address(_) => {}
        }
        report.emit(global.mode())
    }
}

pub async fn open_vault(
    global: &GlobalArgs,
    instance: &Instance,
    profile: &ProfileRecord,
    phrase: &Mnemonic,
    endpoint: Arc<dyn NetworkEndpoint>,
) -> anyhow::Result<RailgunVault> {
    let indexer = instance.indexer_config().await?;
    let vault = RailgunVault::open(
        phrase,
        profile.profile_index,
        instance.network().id.0,
        instance.railgun_database(profile),
        endpoint,
        indexer.effective_url(),
    )?;
    Ok(if let Some(url) = &global.railgun_artifact_url {
        vault.with_artifact_base_url(url.clone())
    } else {
        vault
    })
}

async fn resolve_asset(
    instance: &Instance,
    profile: &ProfileRecord,
    vault: &RailgunVault,
    selector: &str,
) -> anyhow::Result<Asset> {
    if selector.eq_ignore_ascii_case("ETH") {
        return Ok(Asset {
            id: AssetId::Native,
            token: vault.chain().wrapped_base_token,
            symbol: "ETH".into(),
            decimals: 18,
        });
    }
    if selector.eq_ignore_ascii_case("WETH")
        || selector
            .parse::<Address>()
            .is_ok_and(|a| a == vault.chain().wrapped_base_token)
    {
        return Ok(Asset {
            id: AssetId::Erc20(vault.chain().wrapped_base_token),
            token: vault.chain().wrapped_base_token,
            symbol: "WETH".into(),
            decimals: 18,
        });
    }
    let tokens = instance.profile_erc20s(profile).await?;
    let matches: Vec<&Erc20Asset> = tokens
        .iter()
        .filter(|token| {
            token.symbol.eq_ignore_ascii_case(selector)
                || selector
                    .parse::<Address>()
                    .is_ok_and(|a| a == token.address)
        })
        .collect();
    match matches.as_slice() {
        [token] => Ok(Asset {
            id: AssetId::Erc20(token.address),
            token: token.address,
            symbol: token.symbol.clone(),
            decimals: token.decimals,
        }),
        [] => Err(failure::usage(format!(
            "untracked Railgun asset {selector}; use `edw asset opt-in <token-address>`"
        ))),
        _ => Err(failure::usage("ambiguous token symbol; use its address")),
    }
}
fn amount(
    formatted: Option<&str>,
    raw: Option<&str>,
    input: Input,
    asset: &Asset,
) -> anyhow::Result<U256> {
    let amount = match (formatted, raw) {
        (Some(s), None) => parse_amount(s, asset.decimals)?,
        (None, Some(s)) => edw_core::vault::tornadocash::parse_wei(s)?,
        (None, None) => {
            let text = input
                .line(&format!("Amount ({}): ", asset.symbol))?
                .filter(|s| !s.is_empty())
                .ok_or_else(|| failure::usage("pass --amount-formatted or --amount-raw"))?;
            parse_amount(&text, asset.decimals)?
        }
        _ => return Err(failure::usage("pass one amount flag")),
    };
    if amount.is_zero() {
        return Err(failure::usage("amount must be greater than zero"));
    }
    Ok(amount)
}
fn public_calls(calls: &[Call]) -> Vec<PublicCall> {
    calls
        .iter()
        .map(|c| PublicCall {
            target: c.target.to_string(),
            data: c.data.to_string(),
            value: c.value.to_string(),
        })
        .collect()
}
fn poi_report(report: &edw_core::vault::railgun::PoiReport) -> serde_json::Value {
    serde_json::to_value(report).unwrap_or_default()
}

async fn sync(
    session: &mut RailgunSession<'_>,
    submit: bool,
) -> anyhow::Result<Option<serde_json::Value>> {
    eprintln!("Syncing Railgun notes, TXIDs and PPOI lists...");
    let report = session.sync(submit).await?;
    if let Some(report) = &report {
        for failure in &report.failed {
            eprintln!("PPOI {}: {}", failure.txid, failure.error);
        }
        if !report.waiting_for_txid.is_empty() || !report.waiting_for_input_poi.is_empty() {
            eprintln!(
                "Some PPOI proofs are waiting for TXID/input validation; rerun `edw railgun sync` later."
            );
        }
    }
    Ok(report.as_ref().map(poi_report))
}

pub async fn shield(global: &GlobalArgs, args: &crate::shield::ShieldArgs) -> anyhow::Result<()> {
    if args.denominations.is_some() {
        return Err(failure::usage(
            "Railgun uses variable amounts; omit --denominations",
        ));
    }
    if !args.prepare {
        args.consent.ensure_possible(global.input())?;
    }
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), global.input()).await?;
    let phrase = mnemonic(&instance, &profile).await?;
    let endpoint = instance.rpc().await?;
    let vault = open_vault(global, &instance, &profile, &phrase, endpoint.clone()).await?;
    let asset = resolve_asset(&instance, &profile, &vault, &args.token).await?;
    let amount = amount(
        args.amount_formatted.as_deref(),
        args.amount_raw.as_deref(),
        global.input(),
        &asset,
    )?;
    let calls = vault.deposit(&asset.id, amount)?;
    let mut report = RailgunReport::new("shield", &vault);
    report.token = Some(asset.symbol.clone());
    report.amount_raw = Some(amount.to_string());
    if args.prepare {
        report.action = "shield-prepared";
        report.calls = public_calls(&calls);
        return report.emit(global.mode());
    }
    let payer = account_at(&phrase, profile.profile_index, args.from.unwrap_or(0))?;
    let summary = format!(
        "Shield {} {} from {} into {}. Public transaction gas is paid in ETH by that account; ERC-20 approval is mined before shielding.",
        format_amount(amount, asset.decimals)?,
        asset.symbol,
        payer.address,
        vault.address()
    );
    if !confirm(global.input(), &args.consent, &summary)? {
        declined();
        return Ok(());
    }
    if let AssetId::Erc20(token) = asset.id {
        let held = edw_core::asset::erc20_balance(endpoint.as_ref(), token, payer.address).await?;
        if held < amount {
            anyhow::bail!("insufficient {} balance", asset.symbol)
        }
    }
    for call in &calls {
        match send_one(endpoint.as_ref(), payer.signer.clone(), call).await {
            Ok(hash) => {
                eprintln!("Railgun shield transaction {hash}");
                report.transactions.push(hash);
            }
            Err(error) => anyhow::bail!(
                "{error}; completed transactions: {}. Check balances before retrying.",
                report.transactions.join(", ")
            ),
        }
    }
    report.emit(global.mode())
}

pub async fn unshield(
    global: &GlobalArgs,
    args: &crate::unshield::UnshieldArgs,
) -> anyhow::Result<()> {
    if !args.quote {
        args.consent.ensure_possible(global.input())?;
    }
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), global.input()).await?;
    let phrase = mnemonic(&instance, &profile).await?;
    let endpoint = instance.rpc().await?;
    let vault = open_vault(global, &instance, &profile, &phrase, endpoint.clone()).await?;
    let asset = resolve_asset(&instance, &profile, &vault, &args.token).await?;
    let amount = amount(
        args.amount_formatted.as_deref(),
        args.amount_raw.as_deref(),
        global.input(),
        &asset,
    )?;
    let mut calls = args
        .tail_calls
        .as_deref()
        .map(parse_tail_calls)
        .transpose()
        .map_err(failure::usage)?
        .unwrap_or_default();
    let mut total = U256::ZERO;
    for call in &calls {
        total = total
            .checked_add(call.value)
            .ok_or_else(|| failure::usage("tail call value overflow"))?;
    }
    if (asset.id == AssetId::Native && total > amount)
        || (asset.id != AssetId::Native && !total.is_zero())
    {
        return Err(failure::usage(
            "tail call ETH values must fit within a native ETH unshield",
        ));
    }
    let to = args
        .sweep_to
        .as_deref()
        .or(args.to.as_deref())
        .map(recipient_address)
        .transpose()?;
    if args.sweep_to.is_some() && asset.id != AssetId::Native {
        return Err(failure::usage("--sweep-to supports ETH unshields only"));
    }
    if let Some(to) = to {
        calls.push(forward_call(&asset, to, amount - total));
    }
    let mut session = vault.session().await?;
    let poi = sync(&mut session, !args.quote).await?;
    let fresh = instance
        .fresh_address(&profile, &phrase, endpoint.as_ref())
        .await?;
    if args
        .expected_fresh_address
        .is_some_and(|a| a != fresh.address)
    {
        return Err(failure::usage(format!(
            "fresh address is {}, different from --expected-fresh-address",
            fresh.address
        )));
    }
    let owner = account_at(&phrase, profile.profile_index, fresh.index)?.key;
    let bundlers = RailgunBundlers::from_urls(vault.chain().id, &instance.bundler_urls().await?)?;
    let movement = Movement::Unshield {
        token: asset.token,
        amount,
        unwrap_native: asset.id == AssetId::Native,
        calls,
    };
    let quote = session.quote(movement, &owner, &bundlers).await?;
    let mut report = RailgunReport::new("unshield", &vault);
    report.recipient = Some(to.unwrap_or(fresh.address).to_string());
    report.poi = poi;
    finish(
        global,
        &instance,
        &profile,
        &vault,
        &mut session,
        &asset,
        amount,
        &fresh,
        &owner,
        &bundlers,
        quote,
        args.quote,
        &args.consent,
        report,
    )
    .await
}

fn forward_call(asset: &Asset, to: Address, amount: U256) -> Call {
    if asset.id == AssetId::Native {
        Call::new(to, alloy_primitives::Bytes::new(), amount)
    } else {
        transfer_call(asset.token, to, amount)
    }
}

pub async fn transfer(
    global: &GlobalArgs,
    args: &crate::transfer::TransferArgs,
    instance: &Instance,
    profile: &ProfileRecord,
) -> anyhow::Result<()> {
    let phrase = mnemonic(instance, profile).await?;
    let endpoint = instance.rpc().await?;
    let vault = open_vault(global, instance, profile, &phrase, endpoint.clone()).await?;
    let asset = resolve_asset(
        instance,
        profile,
        &vault,
        args.asset.as_deref().unwrap_or("ETH"),
    )
    .await?;
    let amount = amount(args.amount.as_deref(), None, global.input(), &asset)?;
    let to = match &args.to {
        Some(to) => to.clone(),
        None => global
            .input()
            .line("To (0zk address, 0x address or ENS): ")?
            .filter(|s| !s.is_empty())
            .ok_or_else(|| failure::usage("pass --to"))?,
    };
    let mut session = vault.session().await?;
    let poi = sync(&mut session, !args.dry_run).await?;
    let movement = if to.starts_with("0zk") {
        Movement::Transfer {
            token: asset.token,
            amount,
            recipient: to
                .parse()
                .map_err(|e| failure::usage(format!("invalid Railgun address: {e}")))?,
            memo: args.memo.clone().unwrap_or_default(),
        }
    } else {
        if args.memo.is_some() {
            return Err(failure::usage("--memo requires a Railgun 0zk recipient"));
        }
        let recipient: Recipient = to
            .parse()
            .map_err(|e| failure::usage(format!("invalid recipient: {e}")))?;
        let recipient = recipient.resolve(endpoint.as_ref()).await?;
        Movement::Unshield {
            token: asset.token,
            amount,
            unwrap_native: asset.id == AssetId::Native,
            calls: vec![forward_call(&asset, recipient.address, amount)],
        }
    };
    let fresh = instance
        .fresh_address(profile, &phrase, endpoint.as_ref())
        .await?;
    let owner = account_at(&phrase, profile.profile_index, fresh.index)?.key;
    let bundlers = RailgunBundlers::from_urls(vault.chain().id, &instance.bundler_urls().await?)?;
    let quote = session.quote(movement, &owner, &bundlers).await?;
    let mut report = RailgunReport::new("transfer", &vault);
    report.recipient = Some(to);
    report.poi = poi;
    finish(
        global,
        instance,
        profile,
        &vault,
        &mut session,
        &asset,
        amount,
        &fresh,
        &owner,
        &bundlers,
        quote,
        args.dry_run,
        &args.consent,
        report,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish(
    global: &GlobalArgs,
    instance: &Instance,
    profile: &ProfileRecord,
    vault: &RailgunVault,
    session: &mut RailgunSession<'_>,
    asset: &Asset,
    amount: U256,
    fresh: &edw_core::instance::FreshAddress,
    owner: &alloy_signer_local::PrivateKeySigner,
    bundlers: &RailgunBundlers,
    quote: RailgunQuote,
    quote_only: bool,
    consent: &Consent,
    mut report: RailgunReport,
) -> anyhow::Result<()> {
    report.token = Some(
        if report.action == "transfer"
            && report
                .recipient
                .as_ref()
                .is_some_and(|s| s.starts_with("0zk"))
            && asset.id == AssetId::Native
        {
            "WETH".into()
        } else {
            asset.symbol.clone()
        },
    );
    report.amount_raw = Some(amount.to_string());
    report.calls = public_calls(&quote.execution_calls());
    report.fee_raw = Some(quote.fee.to_string());
    report.protocol_fee_raw = Some(quote.protocol_fee.to_string());
    report.fresh_index = Some(fresh.index);
    report.fresh_address = Some(fresh.address.to_string());
    report.quote = quote_only;
    report.bundler = bundlers.first_url().map(str::to_string);
    if quote_only {
        return report.emit(global.mode());
    }
    let mut summary = format!(
        "{} {} {} to {}. Maximum fee {} wei from private WETH, plus protocol fee {} base units of the selected token. Fresh account {} executes this operation with 7702 delegation.",
        report.action,
        format_amount(amount, asset.decimals)?,
        report.token.as_deref().unwrap_or(&asset.symbol),
        report.recipient.as_deref().unwrap_or("the fresh address"),
        quote.fee,
        quote.protocol_fee,
        fresh.address
    );
    for call in &report.calls {
        write!(
            summary,
            "\ncall {}:{}:{}",
            call.target, call.data, call.value
        )?;
    }
    if !confirm(global.input(), consent, &summary)? {
        declined();
        return Ok(());
    }
    eprintln!("Proving Railgun operation...");
    let pending = session.prepare(quote, owner, fresh.index, bundlers).await?;
    keep_fresh(instance, profile, fresh).await;
    eprintln!(
        "Submitting Railgun operation {}",
        pending.signed.hash(vault.chain().id).0
    );
    let result = session.submit_pending(bundlers).await;
    let (hash, tx) = match result {
        Err(error) if session.pending().is_some() => anyhow::bail!(
            "{error}; pending operation {} is preserved. Resume with `edw railgun retry`.",
            pending.signed.hash(vault.chain().id).0
        ),
        result => result?,
    };
    report.user_operation = Some(hash);
    report.transactions.push(tx);
    report.bundler = bundlers.accepted_url();
    report.poi = sync(session, true).await.unwrap_or_else(|e| {
        eprintln!("Operation succeeded; later sync failed: {e:#}");
        None
    });
    report.emit(global.mode())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parser_routes_native_railgun_flows_and_preserves_tail_conflicts() {
        for command in ["shield", "unshield"] {
            assert!(
                crate::Cli::try_parse_from([
                    "edw",
                    command,
                    "--protocol",
                    "railgun",
                    "--token",
                    "WETH",
                    "--amount-formatted",
                    "0.1",
                    "--yes"
                ])
                .is_ok()
            );
        }
        for to in [
            "0x1111111111111111111111111111111111111111",
            "0zk1recipient",
        ] {
            assert!(
                crate::Cli::try_parse_from([
                    "edw",
                    "transfer",
                    "--from",
                    "railgun",
                    "--to",
                    to,
                    "--token",
                    "WETH",
                    "--amount",
                    "1",
                    "--dry-run"
                ])
                .is_ok()
            );
        }
        for command in ["address", "sync", "retry"] {
            assert!(
                crate::Cli::try_parse_from(["edw", "railgun", command, "--profile", "0/0"]).is_ok()
            );
        }
        assert!(
            crate::Cli::try_parse_from([
                "edw",
                "unshield",
                "--protocol",
                "railgun",
                "--tail-calls",
                "0x1111111111111111111111111111111111111111:0x",
                "--to",
                "0x1111111111111111111111111111111111111111"
            ])
            .is_err()
        );
    }

    #[test]
    fn forwarding_uses_native_value_or_exact_token_calldata() {
        let token = Address::repeat_byte(1);
        let to = Address::repeat_byte(2);
        let mut asset = Asset {
            id: AssetId::Native,
            token,
            symbol: "ETH".into(),
            decimals: 18,
        };
        let native = forward_call(&asset, to, U256::from(42));
        assert_eq!(native.target, to);
        assert_eq!(native.value, U256::from(42));
        assert!(native.data.is_empty());
        asset.id = AssetId::Erc20(token);
        assert_eq!(
            forward_call(&asset, to, U256::from(42)),
            transfer_call(token, to, U256::from(42))
        );
    }
}
