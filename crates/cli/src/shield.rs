use std::fmt;

use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;
use clap::{ArgGroup, Args};
use edw_core::{
    broadcast::submit_user_operation,
    bundler::Url,
    call::Call,
    instance::Instance,
    network::NetworkEndpoint,
    vault::tornadocash::{
        Asset, Pool, TornadoVault, format_amount, format_eth, parse_amount, resolve_asset,
        token_address, token_balance,
    },
};
use futures::{StreamExt, TryStreamExt, stream};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    input::Input,
    report::Report,
    spend::{
        Account, Consent, account_at, choose_profile, confirm, declined, mnemonic, open_vault,
        send_one, token_amount,
    },
};

#[derive(Args, Debug)]
#[command(group = ArgGroup::new("amount").args(["amount_formatted", "amount_raw"]))]
pub struct ShieldArgs {
    /// Decimal tokens to shield, for example `1.1`. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_formatted: Option<String>,
    /// Base units to shield. Must be a sum of this chain's Tornado denominations.
    #[arg(long)]
    amount_raw: Option<String>,
    /// Asset symbol: ETH (default), USDC, USDT, DAI, or WBTC.
    #[arg(long, default_value = "ETH")]
    token: String,
    /// Exact pool sizes in the selected token, separated by commas.
    #[arg(long)]
    denominations: Option<String>,
    /// Reserve notes and return deposit calls without checking balances or broadcasting.
    #[arg(long, conflicts_with = "from")]
    prepare: bool,
    /// Standard-address index that pays. Defaults to 0.
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

#[derive(Serialize)]
struct PreparedCall {
    target: String,
    data: String,
    /// Decimal native wei, regardless of the deposited asset.
    value: String,
}

#[derive(Serialize)]
struct PreparedNote {
    nonce: u64,
    pool: String,
    commitment: String,
    amount: String,
}

#[derive(Serialize)]
struct PreparedShieldReport {
    chain_id: u64,
    token: String,
    broadcast: bool,
    notes: Vec<PreparedNote>,
    calls: Vec<PreparedCall>,
    tail_calls: String,
}

impl Report for PreparedShieldReport {
    const KIND: &'static str = "edw/shield-prepared";
    const VERSION: u32 = 1;
}

impl fmt::Display for PreparedShieldReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Reserved {} {} notes locally; not broadcast.",
            self.notes.len(),
            self.token
        )?;
        for note in &self.notes {
            writeln!(
                f,
                "nonce {}: {} {} ({})",
                note.nonce, note.amount, self.token, note.commitment
            )?;
        }
        write!(f, "{}", self.tail_calls)
    }
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
    if !args.prepare {
        args.consent.ensure_possible(input)?;
    }
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let endpoint = instance.rpc().await?;
    let chain_id = endpoint.network_id().await?;
    let asset = resolve_asset(&args.token, chain_id).map_err(failure::usage)?;
    let symbol = asset.symbol().to_uppercase();
    let phrase = mnemonic(&instance, &profile).await?;
    let vault = open_vault(&instance, &profile, &phrase, endpoint.clone()).await?;
    let (pools, amount) = plan_pools(args, input, &asset, &vault).await?;
    if args.prepare {
        let deposits = vault.prepare_deposits(&pools).await?;
        return prepared_report(deposits)?.emit(global.mode());
    }
    let from = match args.from {
        Some(index) => index,
        None if input == Input::Terminal => {
            choose_funded_payer(
                &instance,
                &profile,
                endpoint.as_ref(),
                amount,
                &asset,
                input,
            )
            .await?
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
                "{} {symbol}",
                format_amount(U256::from(pool.amount_wei), asset.decimals())
            )
        })
        .collect();
    let summary = confirmation(
        &format!("{} {symbol}", format_amount(amount, asset.decimals())),
        &notes,
        from,
        payer.address,
        pools.len() > 1 || edw_core::vault::tornadocash::token_address(&asset).is_some(),
    );
    if !confirm(input, &args.consent, &summary)? {
        declined();
        return Ok(());
    }
    check_balances(endpoint.as_ref(), &payer, &asset, amount).await?;
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

async fn plan_pools(
    args: &ShieldArgs,
    input: Input,
    asset: &Asset,
    vault: &TornadoVault<PrivateKeySigner>,
) -> Result<(Vec<Pool>, U256), anyhow::Error> {
    let denominations = args
        .denominations
        .as_deref()
        .map(|raw| parse_denominations(raw, asset))
        .transpose()?;
    let requested = if denominations.is_none()
        || args.amount_formatted.is_some()
        || args.amount_raw.is_some()
    {
        Some(token_amount(
            args.amount_formatted.as_deref(),
            args.amount_raw.as_deref(),
            input,
            asset,
        )?)
    } else {
        None
    };
    let pools = if let Some(denominations) = &denominations {
        vault
            .deposit_pools_for_asset_denominations(denominations, asset)
            .await?
    } else {
        vault
            .deposit_pools_for_asset(
                requested.ok_or_else(|| failure::usage("shield amount is required"))?,
                asset,
            )
            .await?
    };
    let amount: U256 = pools.iter().map(|pool| U256::from(pool.amount_wei)).sum();
    if let Some(requested) = requested
        && requested != amount
    {
        anyhow::bail!(
            "--denominations sum to {} {}, but the requested shield amount is {} {}",
            format_amount(amount, asset.decimals()),
            asset.symbol().to_uppercase(),
            format_amount(requested, asset.decimals()),
            asset.symbol().to_uppercase()
        );
    }
    Ok((pools, amount))
}

async fn check_balances(
    endpoint: &dyn NetworkEndpoint,
    payer: &Account,
    asset: &Asset,
    total: U256,
) -> Result<(), anyhow::Error> {
    if let Some(token) = token_address(asset) {
        let held = token_balance(endpoint, token, payer.address).await?;
        if held < total {
            anyhow::bail!(
                "insufficient {}: have {}, need {}",
                asset.symbol().to_uppercase(),
                format_amount(held, asset.decimals()),
                format_amount(total, asset.decimals())
            );
        }
    }
    let eth_required = if token_address(asset).is_none() {
        total
    } else {
        U256::ZERO
    };
    let balance = endpoint.balance(payer.address).await?;
    if balance <= eth_required {
        anyhow::bail!(
            "{} has {} ETH, which does not cover {} ETH plus gas",
            payer.address,
            format_eth(balance),
            format_eth(eth_required)
        );
    }
    Ok(())
}

fn parse_denominations(raw: &str, asset: &Asset) -> Result<Vec<U256>, anyhow::Error> {
    if raw.is_empty() {
        anyhow::bail!("--denominations needs one to five comma-separated pool sizes");
    }
    raw.split(',')
        .map(|part| {
            if part.trim().is_empty() {
                anyhow::bail!("--denominations contains an empty pool size");
            }
            Ok(parse_amount(part.trim(), asset.decimals())?)
        })
        .collect()
}

async fn choose_funded_payer(
    instance: &Instance,
    profile: &edw_core::profile::ProfileRecord,
    endpoint: &dyn NetworkEndpoint,
    amount: U256,
    asset: &Asset,
    input: Input,
) -> Result<u32, anyhow::Error> {
    let token = token_address(asset);
    let addresses = instance.stored_address_accounts(profile).await?;
    let balances: Vec<(u32, U256, U256)> = stream::iter(addresses)
        .map(|account| async move {
            let eth = endpoint.balance(account.address).await?;
            let held = if let Some(token) = token {
                token_balance(endpoint, token, account.address).await?
            } else {
                eth
            };
            Ok::<_, anyhow::Error>((account.index, eth, held))
        })
        .buffered(8)
        .try_collect()
        .await?;
    let mut funded: Vec<u32> = balances
        .into_iter()
        .filter_map(|(index, eth, held)| {
            (eth > if token.is_some() { U256::ZERO } else { amount } && held >= amount)
                .then_some(index)
        })
        .collect();
    funded.sort_unstable();
    funded.dedup();
    let Some(&default) = funded.first() else {
        anyhow::bail!(
            "no stored address has enough {} and ETH for gas; fund an address or pass --from <index>",
            asset.symbol().to_uppercase()
        );
    };
    let listed = funded
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let entered = input
        .line(&format!(
            "Pay from address index [{listed}] (default {default}): "
        ))?
        .unwrap_or_default();
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

fn prepared_report(
    deposits: Vec<edw_core::vault::tornadocash::PreparedDeposit>,
) -> Result<PreparedShieldReport, anyhow::Error> {
    let first = deposits
        .first()
        .ok_or_else(|| anyhow::anyhow!("shield produced no notes"))?;
    let chain_id = first.pool.chain_id;
    let token = first.pool.symbol().to_uppercase();
    let notes = deposits
        .iter()
        .map(|deposit| PreparedNote {
            nonce: deposit.nonce,
            pool: deposit.pool.id(),
            commitment: deposit.commitment.to_string(),
            amount: format_amount(
                U256::from(deposit.pool.amount_wei),
                deposit.pool.asset.decimals(),
            ),
        })
        .collect();
    let calls: Vec<_> = deposits
        .into_iter()
        .flat_map(|deposit| deposit.calls)
        .map(|call| PreparedCall {
            target: call.target.to_string(),
            data: call.data.to_string(),
            value: call.value.to_string(),
        })
        .collect();
    let tail_calls = calls
        .iter()
        .map(|call| format!("{}:{}:{}", call.target, call.data, call.value))
        .collect::<Vec<_>>()
        .join(",");
    Ok(PreparedShieldReport {
        chain_id,
        token,
        broadcast: false,
        notes,
        calls,
        tail_calls,
    })
}

fn confirmation(
    amount: &str,
    notes: &[String],
    from: u32,
    payer: Address,
    batched: bool,
) -> String {
    let movement = format!(
        "Shield {amount} as {} from {from} ({payer}).",
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use alloy_primitives::{B256, Bytes};
    use clap::Parser;
    use edw_core::vault::tornadocash::{Pool, PreparedDeposit};

    use super::*;

    #[test]
    fn explicit_token_denominations_keep_the_requested_units() {
        let parsed = parse_denominations("100,1000", &Asset::ETHEREUM_USDC).unwrap();
        assert_eq!(
            parsed,
            vec![U256::from(100_000_000_u64), U256::from(1_000_000_000_u64)]
        );
        assert!(
            crate::Cli::try_parse_from([
                "edw",
                "shield",
                "--token",
                "USDC",
                "--denominations",
                "100,1000",
                "--yes"
            ])
            .is_ok()
        );
    }

    #[test]
    fn prepare_needs_no_from_and_returns_roundtrippable_public_calls() {
        let parsed = crate::Cli::try_parse_from([
            "edw",
            "shield",
            "--prepare",
            "--token",
            "USDC",
            "--amount-formatted",
            "100",
            "--non-interactive",
        ])
        .unwrap();
        let crate::Command::Shield(args) = parsed.command else {
            panic!("expected shield");
        };
        assert!(args.prepare);
        assert_eq!(args.from, None);
        let pool = Pool::ETHEREUM_USDC_100;
        let calls = vec![
            Call::new(
                Address::repeat_byte(0x11),
                Bytes::from_static(&[0xaa, 0xbb]),
                U256::ZERO,
            ),
            Call::new(pool.address, Bytes::from_static(&[0xcc, 0xdd]), U256::ZERO),
        ];
        let report = prepared_report(vec![PreparedDeposit {
            pool,
            nonce: 42,
            commitment: B256::repeat_byte(0x22),
            calls: calls.clone(),
        }])
        .unwrap();
        assert_eq!(
            crate::spend::parse_tail_calls(&report.tail_calls).unwrap(),
            calls
        );
        let json: serde_json::Value =
            serde_json::from_str(&report.format(crate::report::Mode::Json).unwrap()).unwrap();
        assert_eq!(json["kind"], "edw/shield-prepared");
        assert_eq!(json["broadcast"], false);
        assert_eq!(json["notes"][0]["nonce"], 42);
        assert_eq!(json["calls"][0]["value"], "0");
        assert!(json.get("secret").is_none());
    }

    #[test]
    fn prepare_rejects_a_payer_selector() {
        assert!(crate::Cli::try_parse_from(["edw", "shield", "--prepare", "--from", "0"]).is_err());
    }
}
