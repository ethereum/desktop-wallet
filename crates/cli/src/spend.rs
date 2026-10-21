use std::sync::Arc;

use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;
use anyhow::Context;
use edw_core::{
    broadcast::submit_call,
    bundler::{BundlerList, bundler_urls},
    call::Call,
    instance::Instance,
    mnemonic::Mnemonic,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    signer::{Signer, simple::SimpleSigner},
    vault::tornadocash::{
        PaymasterWithdrawal, PaymasterWithdrawalRequest, TornadoNote, TornadoVault, format_eth,
        parse_eth, parse_wei, submit_paymaster_withdrawal,
    },
};
use zeroize::Zeroizing;

use crate::input::Input;

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

pub fn first_bundler(chain_id: u64) -> Result<String, anyhow::Error> {
    let urls = bundler_urls(chain_id)?;
    urls.first()
        .map(ToString::to_string)
        .context("no bundler configured")
}

/// Proves `notes` into one paymaster user operation and submits it. The caller stores the fresh
/// address after this returns, then waits for the receipt.
pub async fn submit_shielded(
    withdrawal: ShieldedWithdrawal<'_>,
) -> Result<PaymasterWithdrawal, anyhow::Error> {
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
    let bundlers = BundlerList::from_urls(chain_id, &bundler_urls(chain_id)?)?;
    Ok(submit_paymaster_withdrawal(PaymasterWithdrawalRequest {
        vault: withdrawal.vault,
        provider,
        owner,
        notes: withdrawal.notes,
        forward_to: withdrawal.forward_to,
        minimum_forward: withdrawal.minimum_forward,
        chain_id,
        bundlers,
    })
    .await?)
}
