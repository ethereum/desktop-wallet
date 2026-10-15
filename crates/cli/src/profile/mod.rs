use std::{
    fmt,
    io::{self, IsTerminal, Read},
    path::PathBuf,
};

use anyhow::Context;
use clap::{ArgGroup, Args, Subcommand};
use edw_core::{instance::Instance, profile::ProfileRecord};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{GlobalArgs, input::Input, report::Report, secret_file, session::SessionFile, unlock};

const PHRASE_MAX_BYTES: u64 = 4096;

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
    /// Show the recovery phrase behind a profile. Runs only at a terminal.
    RevealPhrase(RevealPhraseArgs),
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
    long_phrase: bool,
    /// Profile index to create. Defaults to 0 on a new phrase, else the smallest unused.
    #[arg(long)]
    index: Option<u32>,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    #[arg(long)]
    name: Option<String>,
    /// Read the recovery phrase from this file instead of stdin. It must be owned by you with
    /// mode 0400 or 0600.
    #[arg(long, value_name = "PATH")]
    phrase_file: Option<PathBuf>,
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

#[derive(Args, Debug)]
pub struct RevealPhraseArgs {
    /// Profile as `mnemonic/profile` or a unique name. Prompted for when more than one exists.
    selector: Option<String>,
}

#[derive(Serialize)]
pub struct ProfileSummary {
    mnemonic_index: u32,
    profile_index: u32,
    name: Option<String>,
    #[serde(skip)]
    display_name: String,
}

#[derive(Serialize)]
struct ProfileListReport {
    profiles: Vec<ProfileSummary>,
}

#[derive(Serialize)]
struct NewReport {
    profile: ProfileSummary,
    new_phrase: bool,
}

#[derive(Serialize)]
struct ScannedAddress {
    index: u32,
    address: String,
}

#[derive(Serialize)]
struct ImportReport {
    profile: ProfileSummary,
    addresses: Vec<ScannedAddress>,
    next_unused: Option<u32>,
}

#[derive(Serialize)]
struct RenameReport {
    profile: ProfileSummary,
}

impl From<&ProfileRecord> for ProfileSummary {
    fn from(profile: &ProfileRecord) -> Self {
        Self {
            mnemonic_index: profile.mnemonic_index,
            profile_index: profile.profile_index,
            name: profile.name.clone(),
            display_name: profile.display_name(),
        }
    }
}

impl fmt::Display for ProfileSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name)
    }
}

impl Report for ProfileListReport {
    const KIND: &'static str = "edw/profile-list";
    const VERSION: u32 = 1;
}

impl fmt::Display for ProfileListReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.profiles.is_empty() {
            return write!(
                f,
                "No profiles. Create one with `edw profile new` or `edw profile import`."
            );
        }
        let names: Vec<&str> = self
            .profiles
            .iter()
            .map(|profile| profile.display_name.as_str())
            .collect();
        write!(f, "{}", names.join("\n"))
    }
}

impl Report for NewReport {
    const KIND: &'static str = "edw/profile-new";
    const VERSION: u32 = 1;
}

impl fmt::Display for NewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Created profile {}.", self.profile)
    }
}

impl Report for ImportReport {
    const KIND: &'static str = "edw/profile-import";
    const VERSION: u32 = 2;
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Imported profile {}.", self.profile)?;
        if !self.addresses.is_empty() {
            let addresses: Vec<String> = self
                .addresses
                .iter()
                .map(|scanned| format!("{}: {}", scanned.index, scanned.address))
                .collect();
            write!(f, "\nimporting addresses {}", addresses.join(" "))?;
        }
        if let Some(next_unused) = self.next_unused {
            write!(f, "\nnext unused eoa index: {next_unused}")?;
        }
        Ok(())
    }
}

impl Report for RenameReport {
    const KIND: &'static str = "edw/profile-rename";
    const VERSION: u32 = 1;
}

impl fmt::Display for RenameReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Renamed profile to {}.", self.profile)
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List(args) => args.run(global).await,
            Self::New(args) => args.run(global).await,
            Self::Import(args) => args.run(global).await,
            Self::Rename(args) => args.run(global).await,
            Self::RevealPhrase(args) => args.run(global).await,
        }
    }
}

impl ListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let mut profiles = global.open().await?.profiles().await?;
        profiles.sort_by_key(ProfileRecord::key);
        ProfileListReport {
            profiles: profiles.iter().map(ProfileSummary::from).collect(),
        }
        .emit(global.mode())
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
        if self.long_phrase && shared.is_some() {
            anyhow::bail!("--long-phrase only applies to a new recovery phrase");
        }
        let name = prompt_profile_name(self.name.clone(), input)?;

        let profile = if let Some(mnemonic_index) = shared {
            instance
                .add_profile(mnemonic_index, self.index, name)
                .await?
        } else {
            let (mnemonic, profile) = instance
                .generate_profile(self.long_phrase, self.index.unwrap_or(0), name)
                .await?;
            if global.non_interactive || !io::stdout().is_terminal() {
                eprintln!(
                    "The recovery phrase was not shown. Run `edw profile reveal-phrase` at a terminal to back it up."
                );
            } else {
                println!("Write this recovery phrase down now.");
                println!();
                println!("{}", mnemonic.phrase);
                println!();
            }
            profile
        };
        NewReport {
            profile: ProfileSummary::from(&profile),
            new_phrase: shared.is_none(),
        }
        .emit(global.mode())
    }
}

impl ImportArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let input = global.input();
        let phrase = self.phrase(input)?;
        let name = prompt_profile_name(self.name.clone(), input)?;
        let (_, profile) = instance.import_profile(phrase, self.index, name).await?;

        let scan = match instance.endpoint(global.rpc_url.as_deref()).await {
            Ok(endpoint) => instance.scan_profile(&profile, endpoint.as_ref()).await,
            Err(error) => Err(error),
        };
        if let Ok(scan) = &scan {
            instance
                .add_address_vaults(&profile, &scan.addresses)
                .await?;
        }
        let (addresses, next_unused) = match scan {
            Ok(scan) => (
                scan.addresses
                    .iter()
                    .map(|(index, address)| ScannedAddress {
                        index: *index,
                        address: address.to_string(),
                    })
                    .collect(),
                Some(scan.next_unused),
            ),
            Err(error) => {
                eprintln!("The address scan failed: {:#}", anyhow::Error::from(error));
                (Vec::new(), None)
            }
        };
        ImportReport {
            profile: ProfileSummary::from(&profile),
            addresses,
            next_unused,
        }
        .emit(global.mode())
    }

    fn phrase(&self, input: Input) -> Result<Zeroizing<String>, anyhow::Error> {
        if let Some(path) = &self.phrase_file {
            return secret_file::read(path, "recovery phrase", PHRASE_MAX_BYTES);
        }
        if input != Input::Piped {
            return input
                .secret("Recovery phrase: ")?
                .context("no recovery phrase; pass --phrase-file or pipe it on stdin");
        }

        let mut phrase = Zeroizing::new(String::new());
        io::stdin()
            .lock()
            .take(PHRASE_MAX_BYTES + 1)
            .read_to_string(&mut phrase)
            .context("error reading stdin")?;
        if phrase.len() as u64 > PHRASE_MAX_BYTES {
            anyhow::bail!(
                "the recovery phrase on stdin is too large (maximum {PHRASE_MAX_BYTES} bytes)"
            );
        }
        Ok(phrase)
    }
}

impl RenameArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let record = global
            .open()
            .await?
            .rename_profile(&self.selector, Some(self.new_name.clone()))
            .await?;
        RenameReport {
            profile: ProfileSummary::from(&record),
        }
        .emit(global.mode())
    }
}

impl RevealPhraseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        if global.non_interactive {
            anyhow::bail!("profile reveal-phrase never prints the phrase with --non-interactive");
        }
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            anyhow::bail!("profile reveal-phrase only runs at a terminal");
        }

        let input = Input::Terminal;
        let data_dir = global.data_dir();
        let network = match SessionFile::runtime().load().await {
            Some(session) if session.data_dir == data_dir.path() => session.network,
            _ => unlock::choose_network(&data_dir, input)?,
        };
        // An unlocked session alone must not be enough to read the phrase.
        let password = input
            .secret("Decryption password: ")?
            .context("no decryption password")?;
        let instance = Instance::open(&data_dir, network, password.as_bytes()).await?;

        let mut profiles = instance.profiles().await?;
        profiles.sort_by_key(ProfileRecord::key);
        let profile = match &self.selector {
            Some(selector) => instance.profile(selector).await?,
            None => choose_profile(&profiles, input)?,
        };
        let sharing: Vec<String> = profiles
            .iter()
            .filter(|other| other.mnemonic_index == profile.mnemonic_index)
            .map(ProfileRecord::display_name)
            .collect();

        eprintln!(
            "Anyone who sees this phrase can take the funds of: {}.",
            sharing.join(", ")
        );
        let answer = input
            .line(&format!(
                "Show the recovery phrase of {}? [y/N] ",
                profile.display_name()
            ))?
            .unwrap_or_default();
        if !answer.eq_ignore_ascii_case("y") && !answer.eq_ignore_ascii_case("yes") {
            eprintln!("Not shown.");
            return Ok(());
        }

        let phrase = instance.recovery_phrase(&profile).await?;
        println!();
        println!("{}", phrase.as_str());
        println!();
        println!("Store it offline, then clear your terminal scrollback.");
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
    sorted.sort_by_key(|profile| profile.key());
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

pub fn choose_profile(
    profiles: &[ProfileRecord],
    input: Input,
) -> Result<ProfileRecord, anyhow::Error> {
    match profiles {
        [] => {
            anyhow::bail!("no profiles; create one with `edw profile new` or `edw profile import`")
        }
        [only] => Ok(only.clone()),
        several => {
            let labels: Vec<String> = several.iter().map(ProfileRecord::display_name).collect();
            let choice = input
                .choose("Select a profile:", &labels)?
                .context("name the profile as `mnemonic/profile` or by name")?;
            Ok(several[choice].clone())
        }
    }
}
