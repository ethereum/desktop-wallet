use std::sync::Arc;

use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;
use anyhow::Context;
use clap::Args;
use edw_core::{
    broadcast::submit_call,
    bundler::{BundlerList, Url},
    call::Call,
    instance::{FreshAddress, Instance},
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    signer::{Signer, simple::SimpleSigner},
    vault::tornadocash::{
        Asset, IncludedWithdrawal, PaymasterWithdrawal, PaymasterWithdrawalRequest,
        QuotedWithdrawal, TornadoNote, TornadoVault, WithdrawalCost, format_eth, parse_amount,
        parse_wei, quote_paymaster_withdrawal,
    },
};
use zeroize::Zeroizing;

use crate::{failure, input::Input};

pub struct Account {
    pub index: u32,
    pub address: Address,
    pub signer: Arc<dyn Signer>,
    pub key: PrivateKeySigner,
}

pub struct ShieldedWithdrawal<'a> {
    pub phrase: &'a Mnemonic,
    pub profile_index: u32,
    pub endpoint: &'a dyn NetworkEndpoint,
    pub vault: &'a TornadoVault<PrivateKeySigner>,
    pub fresh_index: u32,
    pub notes: &'a [TornadoNote],
    pub forward_to: Option<Address>,
    pub tail_calls: &'a [Call],
    pub asset: &'a Asset,
    pub minimum_forward: Option<U256>,
    pub bundler_urls: &'a [Url],
}

#[derive(Args, Debug)]
pub struct Consent {
    /// Broadcast without asking. Without a terminal, nothing is broadcast unless this is set.
    #[arg(long)]
    yes: bool,
}

impl Consent {
    pub fn ensure_possible(&self, input: Input) -> Result<(), anyhow::Error> {
        if self.yes || input == Input::Terminal {
            Ok(())
        } else {
            Err(failure::usage(
                "nothing was broadcast; without a terminal to confirm at, pass --yes",
            ))
        }
    }
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
                .ok_or_else(|| failure::usage("pass --profile <profile>"))?;
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
    let signer = SimpleSigner::from_key(key.clone());
    let key = PrivateKeySigner::from_signing_key(key);
    Ok(Account {
        index,
        address: signer.address(),
        signer: Arc::new(signer),
        key,
    })
}

pub fn account_by_address(
    mnemonic: &Mnemonic,
    profile_index: u32,
    address: &str,
) -> Result<Account, anyhow::Error> {
    let address = parse_address(address)?;
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

pub fn parse_address(text: &str) -> Result<Address, anyhow::Error> {
    edw_core::recipient::parse_address(text).map_err(|error| failure::usage(error.to_string()))
}

pub fn recipient_address(text: &str) -> Result<Address, anyhow::Error> {
    edw_core::recipient::recipient_address(text).map_err(|error| failure::usage(error.to_string()))
}

pub async fn open_vault(
    instance: &Instance,
    profile: &ProfileRecord,
    mnemonic: &Mnemonic,
    endpoint: Arc<dyn NetworkEndpoint>,
) -> Result<TornadoVault<alloy_signer_local::PrivateKeySigner>, anyhow::Error> {
    let key = mnemonic.keychain_key(profile.profile_index)?;
    Ok(instance.open_tornado_vault(profile, key, endpoint).await?)
}

pub fn confirm(input: Input, consent: &Consent, summary: &str) -> Result<bool, anyhow::Error> {
    consent.ensure_possible(input)?;
    if consent.yes {
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

pub fn note_eth(note: &TornadoNote) -> String {
    format_eth(U256::from(note.pool.amount_wei))
}

pub fn ensure_paymaster(notes: &[TornadoNote]) -> Result<(), anyhow::Error> {
    let sponsor = notes.first().context("no notes to withdraw")?;
    if sponsor.pool.paymaster.is_none() {
        anyhow::bail!(
            "the {} {} pool has no paymaster, so this withdrawal cannot be bundled",
            sponsor.pool.amount(),
            sponsor.pool.symbol().to_uppercase()
        );
    }
    Ok(())
}

pub fn first_bundler(urls: &[Url]) -> Result<String, anyhow::Error> {
    urls.first()
        .map(ToString::to_string)
        .context("no bundler configured; add one with `edw network bundler-config add --url <url>`")
}

/// Proves `notes` into one paymaster user operation and submits it. The caller stores the fresh
/// address after this returns, then waits for the receipt.
pub async fn submit_shielded(
    withdrawal: ShieldedWithdrawal<'_>,
) -> Result<PaymasterWithdrawal, anyhow::Error> {
    Ok(quote_shielded(withdrawal).await?.submit().await?)
}

pub async fn quote_shielded(
    withdrawal: ShieldedWithdrawal<'_>,
) -> Result<QuotedWithdrawal<'_, PrivateKeySigner>, anyhow::Error> {
    let chain_id = withdrawal.endpoint.network_id().await?;
    let provider = withdrawal
        .endpoint
        .provider()
        .context("this endpoint cannot submit a user operation")?;
    let owner = PrivateKeySigner::from_signing_key(
        withdrawal
            .phrase
            .standard_address_key(withdrawal.fresh_index, withdrawal.profile_index)?,
    );
    let bundlers = BundlerList::from_urls(chain_id, withdrawal.bundler_urls)?;
    Ok(quote_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault: withdrawal.vault,
        provider,
        owner,
        asset: withdrawal.asset,
        tail_calls: withdrawal.tail_calls,
        notes: withdrawal.notes,
        forward_to: withdrawal.forward_to,
        minimum_forward: withdrawal.minimum_forward,
        chain_id,
        bundlers,
    })
    .await?)
}

pub async fn measured(included: &IncludedWithdrawal) -> Option<WithdrawalCost> {
    match included.measure().await {
        Ok(cost) => Some(cost),
        Err(error) => {
            eprintln!("Warning: {error}");
            None
        }
    }
}

pub async fn keep_fresh(instance: &Instance, profile: &ProfileRecord, fresh: &FreshAddress) {
    let stored = async {
        instance.commit_fresh_address(profile, fresh.index).await?;
        instance
            .add_address_vault(profile, fresh.index, fresh.address)
            .await
    };
    if let Err(error) = stored.await {
        eprintln!(
            "Warning: fresh address {} was not stored ({error}); `edw profile rescan` lists it once the operation lands.",
            fresh.address
        );
    }
}

/// Amount flags use the selected token's decimals; raw amounts are base units.
pub fn token_amount(
    formatted: Option<&str>,
    raw: Option<&str>,
    input: Input,
    asset: &Asset,
) -> Result<U256, anyhow::Error> {
    match (formatted, raw) {
        (Some(_), Some(_)) => Err(failure::usage("pass only one amount flag")),
        (Some(value), None) => Ok(parse_amount(value, asset.decimals())?),
        (None, Some(value)) => Ok(parse_wei(value)?),
        (None, None) => {
            let value = input
                .line(&format!("Amount ({}): ", asset.symbol().to_uppercase()))?
                .filter(|s| !s.is_empty())
                .ok_or_else(|| failure::usage("pass --amount-formatted or --amount-raw"))?;
            Ok(parse_amount(&value, asset.decimals())?)
        }
    }
}

/// Same TARGET:CALLDATA[:VALUE] grammar as kohaku-cli. Values are always wei.
pub fn parse_tail_calls(raw: &str) -> Result<Vec<Call>, String> {
    raw.split(',').enumerate().map(|(index, entry)| {
        let parts: Vec<_> = entry.split(':').map(str::trim).collect();
        let invalid = || format!("invalid tail call {index}; expected TARGET:0xCALLDATA[:VALUE], with decimal or 0x wei value");
        if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.is_empty()) { return Err(invalid()); }
        if !parts[0].starts_with("0x") || parts[0].len() != 42 { return Err(invalid()); }
        let target = parts[0].parse().map_err(|_| invalid())?;
        if !parts[1].starts_with("0x") || parts[1].len() % 2 != 0 { return Err(invalid()); }
        let data = parts[1].parse().map_err(|_| invalid())?;
        let value = match parts.get(2) {
            None => U256::ZERO,
            Some(value) if value.starts_with("0x") && value.len() > 2 && value[2..].bytes().all(|b| b.is_ascii_hexdigit()) => U256::from_str_radix(&value[2..], 16).map_err(|_| invalid())?,
            Some(value) => parse_wei(value).map_err(|_| invalid())?,
        };
        Ok(Call::new(target, data, value))
    }).collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tails_preserve_order_calldata_and_decimal_or_hex_wei() {
        let target = "0x1111111111111111111111111111111111111111";
        let calls = parse_tail_calls(&format!(
            " {target}:0x ,{target}:0x1234:42,{target}:0xab:0x2a"
        ))
        .expect("valid tails");
        assert_eq!(calls.len(), 3);
        assert!(calls[0].data.is_empty());
        assert_eq!(calls[0].value, U256::ZERO);
        assert_eq!(calls[1].data.as_ref(), &[0x12, 0x34]);
        assert_eq!(calls[1].value, U256::from(42));
        assert_eq!(calls[2].value, U256::from(42));
    }

    #[test]
    fn tails_reject_malformed_entries_and_overflow() {
        let target = "0x1111111111111111111111111111111111111111";
        for entry in [
            String::new(),
            format!("{target}:0x,"),
            "nope:0x".into(),
            format!("{target}:0x1"),
            format!("{target}:aa"),
            format!("{target}:0xgg"),
            format!("{target}:0x:-1"),
            format!("{target}:0x:1.5"),
            format!("{target}:0x:0x"),
            format!("{target}:0x:0x+1"),
            format!("{}:0x", &target[2..]),
            format!("{target}:0x:1:2"),
            format!("{target}:0x:0x1{}", "0".repeat(64)),
        ] {
            assert!(parse_tail_calls(&entry).is_err(), "accepted {entry}");
        }
    }

    #[test]
    fn command_parser_defaults_to_eth_and_checks_tail_conflicts() {
        use clap::Parser;
        for command in ["shield", "unshield"] {
            assert!(
                crate::Cli::try_parse_from([
                    "edw",
                    command,
                    "--amount-formatted",
                    "100",
                    "--token",
                    "USDC"
                ])
                .is_ok()
            );
            assert!(
                crate::Cli::try_parse_from([
                    "edw",
                    command,
                    "--amount-raw",
                    "100",
                    "--amount-formatted",
                    "100"
                ])
                .is_err()
            );
        }
        assert!(crate::Cli::try_parse_from(["edw", "unshield", "--tail-calls", "nope"]).is_err());
        assert!(
            crate::Cli::try_parse_from([
                "edw",
                "unshield",
                "--tail-calls",
                "0x1111111111111111111111111111111111111111:0x",
                "--to",
                "0x1111111111111111111111111111111111111111"
            ])
            .is_err()
        );
    }
}
