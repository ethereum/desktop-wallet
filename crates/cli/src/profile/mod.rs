use std::{
    fmt,
    io::{self, IsTerminal, Read},
    path::PathBuf,
    time::Duration,
};

use anyhow::Context;
use clap::{ArgGroup, Args, Subcommand};
use edw_core::{instance::Instance, mnemonic::scan::EoaScan, profile::ProfileRecord};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{
    GlobalArgs, failure, input::Input, report::Report, secret_file, session::SessionFile, unlock,
};

const PHRASE_MAX_BYTES: u64 = 4096;

#[derive(Subcommand)]
pub enum Command {
    /// List profiles.
    List(ListArgs),
    /// Create a profile on a new recovery phrase, or on one another profile uses.
    New(NewArgs),
    /// Create a profile from an existing recovery phrase.
    Import(ImportArgs),
    /// Scan a profile's addresses again and add an address vault for each one it is missing.
    Rescan(RescanArgs),
    /// Forget unmined Tornado deposit reservations after syncing their pools. Use after a failed shield; a later mined deposit can be recovered by rescan.
    ClearPending(ClearPendingArgs),
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
    /// Exclude extended Tornado pools for this profile.
    #[arg(long)]
    no_extended_pools: bool,
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
    /// Exclude extended Tornado pools for this profile.
    #[arg(long)]
    no_extended_pools: bool,
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
pub struct RescanArgs {
    /// Profile as `mnemonic/profile` or a unique name. Prompted for when more than one exists.
    selector: Option<String>,
}

#[derive(Args, Debug)]
pub struct ClearPendingArgs {
    /// Profile as `mnemonic/profile` or a unique name. Prompted for when more than one exists.
    selector: Option<String>,
    /// Clear only this Tornado note nonce. Omit to clear all unmined reservations.
    #[arg(long)]
    nonce: Option<u64>,
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
    use_extended_pools: bool,
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
    /// Unspent Tornado notes recovered when the profile reuses a stored phrase.
    tornado_notes: Option<u32>,
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
    /// Unspent Tornado notes recovered from the chain's pools, or `None` when the scan failed.
    tornado_notes: Option<u32>,
}

#[derive(Serialize)]
struct RescanReport {
    profile: ProfileSummary,
    added: Vec<ScannedAddress>,
    /// Unspent Tornado notes recovered from the chain's pools, or `None` when the scan failed.
    tornado_notes: Option<u32>,
}

#[derive(Serialize)]
struct ClearPendingReport {
    profile: ProfileSummary,
    cleared: Vec<PendingNote>,
}

#[derive(Serialize)]
struct PendingNote {
    nonce: u64,
    pool: String,
}

#[derive(Serialize)]
struct RenameReport {
    profile: ProfileSummary,
}

impl ProfileSummary {
    pub async fn load(instance: &Instance, profile: &ProfileRecord) -> anyhow::Result<Self> {
        Ok(Self {
            mnemonic_index: profile.mnemonic_index,
            profile_index: profile.profile_index,
            name: profile.name.clone(),
            use_extended_pools: instance.profile_use_extended_pools(profile).await?,
            display_name: profile.display_name(),
        })
    }
}

impl fmt::Display for ProfileSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name)?;
        if !self.use_extended_pools {
            write!(f, " (canonical pools only)")?;
        }
        Ok(())
    }
}

impl Report for ProfileListReport {
    const KIND: &'static str = "edw/profile-list";
    const VERSION: u32 = 2;
}

impl fmt::Display for ProfileListReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.profiles.is_empty() {
            return write!(
                f,
                "No profiles. Create one with `edw profile new` or `edw profile import`."
            );
        }
        let names: Vec<String> = self.profiles.iter().map(ToString::to_string).collect();
        write!(f, "{}", names.join("\n"))
    }
}

impl Report for NewReport {
    const KIND: &'static str = "edw/profile-new";
    const VERSION: u32 = 3;
}

impl fmt::Display for NewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Created profile {}.", self.profile)?;
        write_tornado_count(f, self.tornado_notes)
    }
}

impl Report for ImportReport {
    const KIND: &'static str = "edw/profile-import";
    const VERSION: u32 = 3;
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
        write_tornado_count(f, self.tornado_notes)
    }
}

impl Report for RescanReport {
    const KIND: &'static str = "edw/profile-rescan";
    const VERSION: u32 = 3;
}

impl Report for ClearPendingReport {
    const KIND: &'static str = "edw/profile-clear-pending";
    const VERSION: u32 = 1;
}

impl fmt::Display for ClearPendingReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cleared {} unmined Tornado deposit reservation(s) for {}.",
            self.cleared.len(),
            self.profile
        )?;
        for note in &self.cleared {
            write!(f, "\nnonce {}  {}", note.nonce, note.pool)?;
        }
        Ok(())
    }
}

impl fmt::Display for RescanReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rescanned profile {}.", self.profile)?;
        if self.added.is_empty() {
            write!(f, "\nno new addresses")?;
        } else {
            let addresses: Vec<String> = self
                .added
                .iter()
                .map(|added| format!("{}: {}", added.index, added.address))
                .collect();
            write!(f, "\nadded addresses {}", addresses.join(" "))?;
        }
        write_tornado_count(f, self.tornado_notes)
    }
}

impl Report for RenameReport {
    const KIND: &'static str = "edw/profile-rename";
    const VERSION: u32 = 2;
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
            Self::Rescan(args) => args.run(global).await,
            Self::ClearPending(args) => args.run(global).await,
            Self::Rename(args) => args.run(global).await,
            Self::RevealPhrase(args) => args.run(global).await,
        }
    }
}

impl ListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let mut profiles = instance.profiles().await?;
        profiles.sort_by_key(ProfileRecord::key);
        let mut summaries = Vec::new();
        for profile in &profiles {
            summaries.push(ProfileSummary::load(&instance, profile).await?);
        }
        ProfileListReport {
            profiles: summaries,
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
            return Err(failure::usage(
                "--long-phrase only applies to a new recovery phrase",
            ));
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
        if self.no_extended_pools {
            instance
                .set_profile_use_extended_pools(&profile, false)
                .await?;
        }
        // A new phrase has never been used, but another wallet may have used a stored one.
        let scan = if shared.is_some() {
            scan(&instance, &profile).await
        } else {
            None
        };
        instance
            .add_scanned_address_vaults(&profile, scan.as_ref())
            .await?;
        // A brand-new phrase has no deposits to find. A stored phrase might.
        let tornado_notes = if shared.is_some() {
            recover_tornado(&instance, &profile).await
        } else {
            None
        };
        NewReport {
            profile: ProfileSummary::load(&instance, &profile).await?,
            new_phrase: shared.is_none(),
            tornado_notes,
        }
        .emit(global.mode())
    }
}

impl ImportArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let input = global.input();
        let phrase = self.phrase(input)?;
        eprintln!("Recovery phrase entered; importing profile...");
        let name = prompt_profile_name(self.name.clone(), input)?;
        let (_, profile) = instance.import_profile(phrase, self.index, name).await?;
        if self.no_extended_pools {
            instance
                .set_profile_use_extended_pools(&profile, false)
                .await?;
        }

        let scan = scan(&instance, &profile).await;
        instance
            .add_scanned_address_vaults(&profile, scan.as_ref())
            .await?;
        let (addresses, next_unused) = match scan {
            Some(scan) => (
                scan.addresses
                    .iter()
                    .map(|(index, address)| ScannedAddress {
                        index: *index,
                        address: address.to_string(),
                    })
                    .collect(),
                Some(scan.next_unused),
            ),
            None => (Vec::new(), None),
        };
        let tornado_notes = recover_tornado(&instance, &profile).await;
        ImportReport {
            profile: ProfileSummary::load(&instance, &profile).await?,
            addresses,
            next_unused,
            tornado_notes,
        }
        .emit(global.mode())
    }

    fn phrase(&self, input: Input) -> Result<Zeroizing<String>, anyhow::Error> {
        if let Some(path) = &self.phrase_file {
            return secret_file::read(path, "recovery phrase", PHRASE_MAX_BYTES);
        }
        if input != Input::Piped {
            return input.secret("Recovery phrase: ")?.ok_or_else(|| {
                failure::usage("no recovery phrase; pass --phrase-file or pipe it on stdin")
            });
        }

        let mut phrase = Zeroizing::new(String::new());
        io::stdin()
            .lock()
            .take(PHRASE_MAX_BYTES + 1)
            .read_to_string(&mut phrase)
            .context("error reading stdin")?;
        if phrase.len() as u64 > PHRASE_MAX_BYTES {
            return Err(failure::usage(format!(
                "the recovery phrase on stdin is too large (maximum {PHRASE_MAX_BYTES} bytes)"
            )));
        }
        Ok(phrase)
    }
}

impl RescanArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let profile = if let Some(selector) = &self.selector {
            instance.profile(selector).await?
        } else {
            let mut profiles = instance.profiles().await?;
            profiles.sort_by_key(ProfileRecord::key);
            choose_profile(&profiles, global.input())?
        };

        let endpoint = instance.rpc().await?;
        let scan = instance.scan_profile(&profile, endpoint.as_ref()).await?;
        let added = instance.add_missing_address_vaults(&profile, &scan).await?;
        let tornado_notes = recover_tornado(&instance, &profile).await;
        RescanReport {
            profile: ProfileSummary::load(&instance, &profile).await?,
            added: added
                .iter()
                .map(|(index, address)| ScannedAddress {
                    index: *index,
                    address: address.to_string(),
                })
                .collect(),
            tornado_notes,
        }
        .emit(global.mode())
    }
}

impl ClearPendingArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let profile = if let Some(selector) = &self.selector {
            instance.profile(selector).await?
        } else {
            let mut profiles = instance.profiles().await?;
            profiles.sort_by_key(ProfileRecord::key);
            choose_profile(&profiles, global.input())?
        };
        let phrase = crate::spend::mnemonic(&instance, &profile).await?;
        let endpoint = instance.rpc().await?;
        let vault = crate::spend::open_vault(&instance, &profile, &phrase, endpoint).await?;
        eprintln!("Syncing Tornado pools before clearing unmined reservations...");
        let cleared = vault
            .clear_pending(self.nonce)
            .await?
            .into_iter()
            .map(|note| PendingNote {
                nonce: note.nonce,
                pool: note.pool.id(),
            })
            .collect();
        ClearPendingReport {
            profile: ProfileSummary::load(&instance, &profile).await?,
            cleared,
        }
        .emit(global.mode())
    }
}

impl RenameArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let record = instance
            .rename_profile(&self.selector, Some(self.new_name.clone()))
            .await?;
        RenameReport {
            profile: ProfileSummary::load(&instance, &record).await?,
        }
        .emit(global.mode())
    }
}

impl RevealPhraseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        if global.non_interactive {
            return Err(failure::usage(
                "profile reveal-phrase never prints the phrase with --non-interactive",
            ));
        }
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(failure::usage(
                "profile reveal-phrase only runs at a terminal",
            ));
        }

        let input = Input::Terminal;
        let data_dir = global.data_dir();
        let network = match SessionFile::runtime().load_if_present().await {
            Some(session) if session.data_dir == data_dir.path() => session.network,
            _ => unlock::choose_network(&data_dir, input)?,
        };
        // An unlocked session alone must not be enough to read the phrase.
        let password = input
            .secret("Decryption password: ")?
            .filter(|password| !password.is_empty())
            .ok_or_else(|| failure::locked("no decryption password"))?;
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

/// Recovers Tornado notes on the current chain, or warns on stderr and returns `None` when the
/// scan fails.
async fn recover_tornado(instance: &Instance, profile: &ProfileRecord) -> Option<u32> {
    let recovered: Result<u32, anyhow::Error> = async {
        let endpoint = instance.rpc().await?;
        let phrase = crate::spend::mnemonic(instance, profile).await?;
        let vault = crate::spend::open_vault(instance, profile, &phrase, endpoint).await?;
        eprintln!("Recovering Tornado notes...");
        let recovery = vault.recover_chain_with_progress(|index, total, pool| {
            eprintln!("Syncing Tornado pool {index}/{total}: {pool}");
        });
        tokio::pin!(recovery);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
        heartbeat.tick().await;
        let count = loop {
            tokio::select! {
                result = &mut recovery => break result?,
                _ = heartbeat.tick() => eprintln!("Still recovering Tornado notes..."),
            }
        };
        Ok(u32::try_from(count)?)
    }
    .await;
    match recovered {
        Ok(count) => Some(count),
        Err(error) => {
            eprintln!("The Tornado scan failed: {error:#}");
            None
        }
    }
}

fn tornado_count_line(count: u32) -> String {
    match count {
        0 => "no unspent tornado notes".to_string(),
        1 => "1 unspent tornado note".to_string(),
        count => format!("{count} unspent tornado notes"),
    }
}

fn write_tornado_count(f: &mut fmt::Formatter<'_>, count: Option<u32>) -> fmt::Result {
    match count {
        Some(count) => write!(f, "\n{}", tornado_count_line(count)),
        None => Ok(()),
    }
}

/// Scans `profile`'s addresses, or warns on stderr that only address 0 gets an address vault and
/// returns `None` when the scan fails.
async fn scan(instance: &Instance, profile: &ProfileRecord) -> Option<EoaScan> {
    eprintln!("Scanning standard addresses...");
    let scan = match instance.rpc().await {
        Ok(endpoint) => {
            let scanning = instance.scan_profile(profile, endpoint.as_ref());
            tokio::pin!(scanning);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    result = &mut scanning => break result,
                    _ = heartbeat.tick() => eprintln!("Still scanning standard addresses..."),
                }
            }
        }
        Err(error) => Err(error),
    };
    scan.map_err(|error| {
        eprintln!(
            "The address scan failed, so only address 0 gets an address vault: {:#}",
            anyhow::Error::from(error)
        );
    })
    .ok()
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
        .ok_or_else(|| failure::usage("pass --new-phrase or --phrase-of <profile>"))?;
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
            let choice = input.choose("Select a profile:", &labels)?.ok_or_else(|| {
                failure::usage("name the profile as `mnemonic/profile` or by name")
            })?;
            Ok(several[choice].clone())
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn creation_and_import_accept_opt_out_but_default_to_extended() {
        use clap::Parser;
        for command in ["new", "import"] {
            for opt_out in [false, true] {
                let mut args = vec!["edw", "profile", command];
                if opt_out {
                    args.push("--no-extended-pools");
                }
                let cli = crate::Cli::try_parse_from(args).expect("valid profile arguments");
                let crate::Command::Profile(profile) = cli.command else {
                    panic!("profile command")
                };
                let selected = match profile {
                    Command::New(args) => args.no_extended_pools,
                    Command::Import(args) => args.no_extended_pools,
                    _ => panic!("creation command"),
                };
                assert_eq!(selected, opt_out);
            }
        }
        assert!(crate::Cli::try_parse_from(["edw", "shield", "--no-extended-pools"]).is_err());
    }

    #[test]
    fn tornado_count_names_how_many_notes_were_found() {
        assert_eq!(tornado_count_line(0), "no unspent tornado notes");
        assert_eq!(tornado_count_line(1), "1 unspent tornado note");
        assert_eq!(tornado_count_line(3), "3 unspent tornado notes");
    }
}
