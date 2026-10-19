use std::sync::Arc;

use alloy_primitives::{Address, U256};
use anyhow::Context;
use edw_core::{
    broadcast::{submit_batch, submit_call},
    call::Call,
    instance::Instance,
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    signer::{Signer, simple::SimpleSigner},
    vault::{
        VaultId,
        tornadocash::{TornadoNote, TornadoVault, format_eth, parse_eth, parse_wei},
    },
};
use zeroize::Zeroizing;

use crate::input::Input;

pub struct Account {
    pub index: u32,
    pub address: Address,
    pub signer: Arc<dyn Signer>,
}

pub async fn choose_profile(
    instance: &Instance,
    selector: Option<&str>,
    input: Input,
) -> Result<ProfileRecord, anyhow::Error> {
    if let Some(selector) = selector {
        return Ok(instance.profile(selector).await?);
    }
    let mut profiles = instance.profiles().await?;
    profiles.sort_by_key(ProfileRecord::key);
    match profiles.as_slice() {
        [] => {
            anyhow::bail!("no profiles; create one with `edw profile new` or `edw profile import`")
        }
        [only] => Ok(only.clone()),
        several => {
            let labels: Vec<String> = several.iter().map(ProfileRecord::display_name).collect();
            let choice = input
                .choose("Select a profile:", &labels)?
                .context("pass --profile <profile>")?;
            Ok(several[choice].clone())
        }
    }
}

pub async fn mnemonic(
    instance: &Instance,
    profile: &ProfileRecord,
) -> Result<Mnemonic, anyhow::Error> {
    let phrase: Zeroizing<String> = instance.recovery_phrase(profile).await?;
    Ok(Mnemonic::parse(phrase.as_str())?)
}

pub fn account_at(
    mnemonic: &Mnemonic,
    profile_index: u32,
    index: u32,
) -> Result<Account, anyhow::Error> {
    let key = mnemonic.standard_address_key(index, profile_index)?;
    let signer = SimpleSigner::from_key(key);
    Ok(Account {
        index,
        address: signer.address(),
        signer: Arc::new(signer),
    })
}

pub fn account_by_address(
    mnemonic: &Mnemonic,
    profile_index: u32,
    address: Address,
) -> Result<Account, anyhow::Error> {
    for index in 0..1000 {
        let account = account_at(mnemonic, profile_index, index)?;
        if account.address == address {
            return Ok(account);
        }
    }
    anyhow::bail!(
        "address {address} is not one of the first 1000 standard addresses of this profile"
    )
}

pub async fn open_vault(
    instance: &Instance,
    profile: &ProfileRecord,
    mnemonic: &Mnemonic,
    endpoint: Arc<dyn NetworkEndpoint>,
) -> Result<TornadoVault<alloy_signer_local::PrivateKeySigner>, anyhow::Error> {
    let key = mnemonic.keychain_key(profile.profile_index)?;
    Ok(TornadoVault::open(key, instance.tornado_database(profile), endpoint).await?)
}

/// `--amount-formatted` as decimal ETH, `--amount-raw` as wei, or a prompted ETH amount.
///
/// The two flags cannot both be set. A missing amount is read as decimal ETH.
pub fn tornado_amount(
    formatted: Option<&str>,
    raw: Option<&str>,
    input: Input,
) -> Result<U256, anyhow::Error> {
    match (formatted, raw) {
        (Some(_), Some(_)) => {
            anyhow::bail!("pass only one of --amount-formatted and --amount-raw")
        }
        (Some(amount), None) => Ok(parse_eth(amount)?),
        (None, Some(amount)) => Ok(parse_wei(amount)?),
        (None, None) => {
            let text = input
                .line("Amount (ETH): ")?
                .filter(|line| !line.is_empty())
                .context("pass --amount-formatted or --amount-raw")?;
            Ok(parse_eth(&text)?)
        }
    }
}

pub fn eth_amount(given: Option<&str>, input: Input, label: &str) -> Result<U256, anyhow::Error> {
    let text = match given {
        Some(amount) => amount.to_string(),
        None => input
            .line(label)?
            .filter(|line| !line.is_empty())
            .context(format!("pass {label}"))?,
    };
    Ok(parse_eth(&text)?)
}

pub fn confirm(input: Input, summary: &str) -> Result<bool, anyhow::Error> {
    if input != Input::Terminal {
        return Ok(true);
    }
    eprintln!("{summary}");
    let answer = input
        .line("Broadcast this transaction? [y/N] ")?
        .unwrap_or_default();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

pub fn declined() {
    eprintln!("Not broadcast.");
}

pub async fn send_one(
    endpoint: &dyn NetworkEndpoint,
    signer: Arc<dyn Signer>,
    call: &Call,
) -> Result<String, anyhow::Error> {
    Ok(submit_call(endpoint, signer, call).await?.to_string())
}

pub async fn send_batch(
    endpoint: &dyn NetworkEndpoint,
    context: Arc<dyn Signer>,
    payer: Arc<dyn Signer>,
    calls: &[Call],
) -> Result<String, anyhow::Error> {
    Ok(submit_batch(endpoint, context, payer, calls)
        .await?
        .to_string())
}

pub fn note_eth(note: &TornadoNote) -> String {
    format_eth(U256::from(note.pool.amount_wei))
}

pub async fn require_gas(
    endpoint: &dyn NetworkEndpoint,
    payer: Address,
) -> Result<(), anyhow::Error> {
    let balance = endpoint.balance(payer).await?;
    if balance.is_zero() {
        anyhow::bail!("{payer} has no ETH for gas. Fund it, then run the command again.");
    }
    Ok(())
}

/// Withdrawals to `fresh`, then an optional forward of `forward` wei to `to`.
pub async fn withdrawal_calls(
    vault: &TornadoVault<alloy_signer_local::PrivateKeySigner>,
    notes: &[TornadoNote],
    fresh: Address,
    forward: Option<(Address, U256)>,
) -> Result<Vec<Call>, anyhow::Error> {
    eprintln!(
        "Proving {} note{}. This can take a while.",
        notes.len(),
        if notes.len() == 1 { "" } else { "s" }
    );
    let mut calls = Vec::with_capacity(notes.len() + usize::from(forward.is_some()));
    for (index, note) in notes.iter().enumerate() {
        eprintln!(
            "Proving {} ETH ({}/{})...",
            note_eth(note),
            index + 1,
            notes.len()
        );
        calls.push(vault.withdraw(note, &VaultId::Address(fresh)).await?);
    }
    if let Some((to, amount)) = forward {
        calls.push(Call::new(to, alloy_primitives::Bytes::new(), amount));
    }
    Ok(calls)
}
