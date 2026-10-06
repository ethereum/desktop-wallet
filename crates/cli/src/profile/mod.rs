use anyhow::Context;
use clap::{ArgGroup, Args, Subcommand};
use edw_core::profile::ProfileRecord;
use zeroize::Zeroizing;

use crate::{GlobalArgs, input::Input};

#[derive(Subcommand)]
pub enum Command {
    /// List profiles.
    List(ListArgs),
    /// Create a profile on a new recovery phrase, or on one another profile uses.
    New(NewArgs),
    /// Create a profile from an existing recovery phrase.
    Import(ImportArgs),
    /// Set or clear a profile's optional name.
    Rename(RenameArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {}

#[derive(Args, Debug)]
#[command(group = ArgGroup::new("phrase").args(["new_phrase", "phrase_of"]))]
pub struct NewArgs {
    #[arg(long)]
    name: Option<String>,
    /// Create the profile on a new recovery phrase.
    #[arg(long)]
    new_phrase: bool,
    /// Create the profile on the recovery phrase this profile uses.
    #[arg(long, value_name = "PROFILE")]
    phrase_of: Option<String>,
    /// Generate a 24-word phrase instead of 12.
    #[arg(long, conflicts_with = "phrase_of")]
    long_seed: bool,
    /// Profile index to create. Defaults to 0 on a new phrase, else the smallest unused.
    #[arg(long)]
    index: Option<u32>,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    #[arg(long)]
    name: Option<String>,
    /// Profile index to create. Defaults to 0.
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Args, Debug)]
pub struct RenameArgs {
    /// Profile to rename, by name.
    selector: String,
    /// New name. Pass `-` or an empty string to clear it.
    new_name: String,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List(args) => args.run(global).await,
            Self::New(args) => args.run(global).await,
            Self::Import(args) => args.run(global).await,
            Self::Rename(args) => args.run(global).await,
        }
    }
}

impl ListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let mut profiles = global.open().await?.profiles().await?;
        if profiles.is_empty() {
            println!("No profiles. Create one with `edw profile new` or `edw profile import`.");
            return Ok(());
        }

        profiles.sort_by_key(|profile| (profile.mnemonic_index, profile.profile_index));
        for profile in &profiles {
            println!("{}", profile.display_name());
        }
        Ok(())
    }
}

impl NewArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let input = global.input();
        let shared = if self.new_phrase {
            None
        } else if let Some(selector) = &self.phrase_of {
            Some(instance.profile(selector).await?.mnemonic_index)
        } else {
            choose_phrase(&instance.profiles().await?, input)?
        };
        if self.long_seed && shared.is_some() {
            anyhow::bail!("--long-seed only applies to a new recovery phrase");
        }
        let name = prompt_profile_name(self.name.clone(), input)?;

        let Some(mnemonic_index) = shared else {
            let (mnemonic, profile) = instance
                .generate_profile(self.long_seed, self.index.unwrap_or(0), name)
                .await?;
            println!("Write this recovery phrase down now. It is shown only this once.");
            println!();
            println!("{}", mnemonic.phrase);
            println!();
            println!("Created profile {}.", profile.display_name());
            return Ok(());
        };
        let profile = instance
            .add_profile(mnemonic_index, self.index, name)
            .await?;
        println!("Created profile {}.", profile.display_name());
        Ok(())
    }
}

impl ImportArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let phrase = global
            .input()
            .secret("Recovery phrase: ")?
            .context("no recovery phrase; pipe it on stdin")?;
        let phrase = Zeroizing::new(phrase.split_whitespace().collect::<Vec<_>>().join(" "));
        let name = prompt_profile_name(self.name.clone(), global.input())?;
        let (_, profile) = instance.import_profile(phrase, self.index, name).await?;
        println!("Imported profile {}.", profile.display_name());

        let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
        let scan = instance.scan_profile(&profile, endpoint.as_ref()).await?;
        if !scan.addresses.is_empty() {
            let addresses = scan
                .addresses
                .iter()
                .map(|(index, address)| format!("{index}: {address}"))
                .collect::<Vec<_>>()
                .join(" ");
            println!("importing addresses {addresses}");
        }
        println!("next unused eoa index: {}", scan.next_unused);
        Ok(())
    }
}

impl RenameArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let record = global
            .open()
            .await?
            .rename_profile(&self.selector, Some(self.new_name.clone()))
            .await?;
        println!("Renamed profile to {}.", record.display_name());
        Ok(())
    }
}

/// `explicit` when given, else a name typed at an interactive terminal. Validation is the
/// instance's.
fn prompt_profile_name(
    explicit: Option<String>,
    input: Input,
) -> Result<Option<String>, anyhow::Error> {
    if explicit.is_some() || input != Input::Terminal {
        return Ok(explicit);
    }
    Ok(input
        .line("Profile name: ")?
        .filter(|name| !name.is_empty()))
}

/// `None` for a new recovery phrase, else the stored phrase to create the profile on.
///
/// With no stored phrase there is nothing to pick, so it is a new one.
fn choose_phrase(profiles: &[ProfileRecord], input: Input) -> Result<Option<u32>, anyhow::Error> {
    let mut phrases: Vec<(u32, Vec<String>)> = Vec::new();
    let mut sorted: Vec<&ProfileRecord> = profiles.iter().collect();
    sorted.sort_by_key(|profile| (profile.mnemonic_index, profile.profile_index));
    for profile in sorted {
        match phrases.last_mut() {
            Some((index, names)) if *index == profile.mnemonic_index => {
                names.push(profile.display_name());
            }
            _ => phrases.push((profile.mnemonic_index, vec![profile.display_name()])),
        }
    }
    if phrases.is_empty() {
        return Ok(None);
    }

    let labels: Vec<String> = std::iter::once("a new recovery phrase".to_string())
        .chain(
            phrases
                .iter()
                .map(|(_, names)| format!("the recovery phrase of {}", names.join(", "))),
        )
        .collect();
    let choice = input
        .choose("Create the profile on:", &labels)?
        .context("pass --new-phrase or --phrase-of <profile>")?;
    Ok(choice.checked_sub(1).map(|index| phrases[index].0))
}
