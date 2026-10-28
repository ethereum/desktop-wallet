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
        IncludedWithdrawal, PaymasterWithdrawal, PaymasterWithdrawalRequest, QuotedWithdrawal,
        TornadoNote, TornadoVault, WithdrawalCost, format_eth, parse_eth, parse_wei,
        quote_paymaster_withdrawal,
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
    pub minimum_forward: Option<U256>,
    pub tail_calls: &'a [Call],
    pub bundler_urls: &'a [Url],
}

#[derive(Args, Debug)]
pub struct Consent {
    /// Broadcast without asking. Without a terminal, nothing is broadcast unless this is set.
    #[arg(long)]
    yes: bool,
}

impl Consent {
    /// Fails unless consent can be given: `--yes`, or a terminal to ask at. Call it before any
    /// network work.
    pub fn ensure_possible(&self, input: Input) -> Result<(), anyhow::Error> {
        if self.yes || input == Input::Terminal {
            return Ok(());
        }
        Err(failure::usage(
            "nothing was broadcast; without a terminal to confirm at, pass --yes",
        ))
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
    let indexer = instance.indexer_config().await?;
    Ok(TornadoVault::open(
        key,
        instance.tornado_database(profile),
        endpoint,
        indexer.effective_url(),
    )?)
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
        (Some(_), Some(_)) => Err(failure::usage(
            "pass only one of --amount-formatted and --amount-raw",
        )),
        (Some(amount), None) => Ok(parse_eth(amount)?),
        (None, Some(amount)) => Ok(parse_wei(amount)?),
        (None, None) => {
            let text = input
                .line("Amount (ETH): ")?
                .filter(|line| !line.is_empty())
                .ok_or_else(|| failure::usage("pass --amount-formatted or --amount-raw"))?;
            Ok(parse_eth(&text)?)
        }
    }
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
            "the {} ETH pool has no paymaster, so this withdrawal cannot be bundled",
            note_eth(sponsor)
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
        notes: withdrawal.notes,
        forward_to: withdrawal.forward_to,
        minimum_forward: withdrawal.minimum_forward,
        tail_calls: withdrawal.tail_calls,
        chain_id,
        bundlers,
    })
    .await?)
}

/// The fee and change of `included`, or a warning and `None` when they cannot be measured.
pub async fn measured(included: &IncludedWithdrawal) -> Option<WithdrawalCost> {
    match included.measure().await {
        Ok(cost) => Some(cost),
        Err(error) => {
            eprintln!("Warning: {error}");
            None
        }
    }
}

/// Stores the fresh address as used and as an address vault, so `balance` shows what it holds.
/// Call it after consent and before submitting; a failure only warns.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_terminal_only_yes_consents() {
        for input in [Input::Piped, Input::Disabled] {
            assert!(Consent { yes: false }.ensure_possible(input).is_err());
            assert!(Consent { yes: true }.ensure_possible(input).is_ok());
        }
    }
}
