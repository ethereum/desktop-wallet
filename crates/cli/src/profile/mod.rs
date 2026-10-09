use std::{
    fmt,
    io::{self, BufRead, IsTerminal, Write},
    path::PathBuf,
};

use clap::{Args, Subcommand};
use edw_core::{
    mnemonic::{self, resolve_mnemonic, scan::scan_standard_eoas},
    profile::simple::{
        ProfileRecord, bootstrap_profile, db::SimpleProfileDb, next_profile_index, rename_profile,
    },
};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{GlobalArgs, report::Report, secret_file};

const MNEMONIC_FILE_MAX_BYTES: u64 = 4096;

#[derive(Subcommand)]
pub enum Command {
    /// List profiles grouped by mnemonic.
    List,
    /// Generate a new mnemonic and one profile.
    Generate(GenerateArgs),
    /// Import a mnemonic phrase and one profile.
    Import(ImportArgs),
    /// Add a profile on an existing mnemonic.
    Add(AddArgs),
    /// Set or clear a profile's optional name.
    Rename(RenameArgs),
}

#[derive(Args, Debug)]
pub struct GenerateArgs {
    #[arg(long)]
    name: Option<String>,
    /// Generate a 24-word phrase instead of 12.
    #[arg(long)]
    long_seed: bool,
    /// Profile index to create. Defaults to 0.
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    #[arg(long)]
    name: Option<String>,
    /// Read the phrase from this file instead of stdin. It must be owned by you with mode 0400
    /// or 0600.
    #[arg(long, value_name = "PATH")]
    mnemonic_file: Option<PathBuf>,
    /// Profile index to create. Defaults to 0.
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    #[arg(long)]
    name: Option<String>,
    /// Mnemonic index. Prompted when more than one mnemonic exists.
    #[arg(long)]
    mnemonic: Option<u32>,
    /// Profile index to create. Defaults to 0.
    #[arg(long, default_value_t = 0, conflicts_with = "next")]
    index: u32,
    /// Create the smallest unused profile index on the mnemonic.
    #[arg(long, conflicts_with = "index")]
    next: bool,
}

#[derive(Args, Debug)]
pub struct RenameArgs {
    /// Profile as `mnemonic/profile` or a unique name.
    selector: String,
    /// New name. Pass `-` or an empty string to clear it.
    new_name: String,
}

#[derive(Serialize)]
struct ProfileSummary {
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

/// What `profile generate` created. It never carries the recovery phrase, which
/// `--non-interactive` withholds.
#[derive(Serialize)]
struct GenerateReport {
    mnemonic_index: u32,
    profile: ProfileSummary,
}

#[derive(Serialize)]
struct ScannedAddress {
    index: u32,
    address: String,
}

#[derive(Serialize)]
struct ImportReport {
    mnemonic_index: u32,
    profile: ProfileSummary,
    addresses: Vec<ScannedAddress>,
    /// `None` when the scan failed; the import itself still happened.
    next_unused: Option<u32>,
    scan_error: Option<String>,
}

#[derive(Serialize)]
struct AddReport {
    profile: ProfileSummary,
}

#[derive(Serialize)]
struct RenameReport {
    profile: ProfileSummary,
}

impl From<&ProfileRecord> for ProfileSummary {
    fn from(record: &ProfileRecord) -> Self {
        Self {
            mnemonic_index: record.mnemonic_index,
            profile_index: record.profile_index,
            name: record.name.clone(),
            display_name: record.display_name(),
        }
    }
}

impl fmt::Display for ProfileSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{} ({})",
            self.mnemonic_index, self.profile_index, self.display_name
        )
    }
}

impl Report for ProfileListReport {
    const KIND: &'static str = "edw/profile-list";
    const VERSION: u32 = 1;
}

impl fmt::Display for ProfileListReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.profiles.is_empty() {
            return write!(f, "No profiles.");
        }

        let mut lines = Vec::new();
        let mut current = None;
        for profile in &self.profiles {
            if current != Some(profile.mnemonic_index) {
                current = Some(profile.mnemonic_index);
                lines.push(format!("Mnemonic {}", profile.mnemonic_index));
            }
            lines.push(format!(
                "  {}  {}",
                profile.profile_index, profile.display_name
            ));
        }
        write!(f, "{}", lines.join("\n"))
    }
}

impl Report for GenerateReport {
    const KIND: &'static str = "edw/profile-generate";
    const VERSION: u32 = 1;
}

impl fmt::Display for GenerateReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Created mnemonic {} and profile {}.",
            self.mnemonic_index, self.profile
        )
    }
}

impl Report for ImportReport {
    const KIND: &'static str = "edw/profile-import";
    const VERSION: u32 = 1;
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Imported mnemonic {}; profile {}.",
            self.mnemonic_index, self.profile
        )?;
        if let Some(error) = &self.scan_error {
            return write!(f, "\nThe address scan failed: {error}");
        }
        if !self.addresses.is_empty() {
            let addresses = self
                .addresses
                .iter()
                .map(|scanned| format!("{}: {}", scanned.index, scanned.address))
                .collect::<Vec<_>>()
                .join(" ");
            write!(f, "\nimporting addresses {addresses}")?;
        }
        if let Some(next_unused) = self.next_unused {
            write!(f, "\nnext unused eoa index: {next_unused}")?;
        }
        Ok(())
    }
}

impl Report for AddReport {
    const KIND: &'static str = "edw/profile-add";
    const VERSION: u32 = 1;
}

impl fmt::Display for AddReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Created profile {}.", self.profile)
    }
}

impl Report for RenameReport {
    const KIND: &'static str = "edw/profile-rename";
    const VERSION: u32 = 1;
}

impl fmt::Display for RenameReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Renamed profile {}/{} to {}.",
            self.profile.mnemonic_index, self.profile.profile_index, self.profile.display_name
        )
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List => list(global).await,
            Self::Generate(args) => generate(global, args).await,
            Self::Import(args) => import(global, args).await,
            Self::Add(args) => add(global, args).await,
            Self::Rename(args) => rename(global, args).await,
        }
    }
}

async fn list(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let mnemonics = context.mnemonics().await?;
    let profiles = context.profiles().await?;

    let mut listed = Vec::new();
    for mnemonic in &mnemonics {
        let mut group: Vec<_> = profiles
            .iter()
            .filter(|profile| profile.mnemonic_index == mnemonic.index)
            .collect();
        if group.is_empty() {
            continue;
        }
        group.sort_by_key(|profile| profile.profile_index);
        for profile in group {
            let _pointer = context
                .profile_db(profile.mnemonic_index, profile.profile_index)
                .get_pointer()
                .await?;
            listed.push(ProfileSummary::from(profile));
        }
    }
    ProfileListReport { profiles: listed }.emit(global.mode())
}

async fn generate(global: &GlobalArgs, args: &GenerateArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let profiles = context.profiles().await?;
    let name = prompt_profile_name(
        args.name.clone(),
        !global.non_interactive,
        &profiles,
        args.index,
        None,
    )?;
    let (mnemonic, profile) =
        mnemonic::generate_as_profile(context.store.clone(), args.long_seed, args.index, name)
            .await?;
    if !global.non_interactive {
        println!("Write this recovery phrase down now. It is shown only this once.");
        println!();
        println!("{}", mnemonic.phrase);
        println!();
    }
    GenerateReport {
        mnemonic_index: mnemonic.index,
        profile: ProfileSummary::from(&profile),
    }
    .emit(global.mode())
}

async fn import(global: &GlobalArgs, args: &ImportArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let phrase = match &args.mnemonic_file {
        Some(path) => secret_file::read(path, "mnemonic", MNEMONIC_FILE_MAX_BYTES)?,
        None => read_phrase(global.non_interactive)?,
    };
    let phrase = Zeroizing::new(phrase.split_whitespace().collect::<Vec<_>>().join(" "));
    let profiles = context.profiles().await?;
    let name = prompt_profile_name(
        args.name.clone(),
        !global.non_interactive,
        &profiles,
        args.index,
        None,
    )?;
    let (mnemonic, profile) =
        mnemonic::import_as_profile(context.store.clone(), phrase, args.index, name).await?;

    let scan = async {
        let provider = context.endpoint(global.rpc_url.as_deref()).await?;
        let parsed = mnemonic.mnemonic()?;
        anyhow::Ok(scan_standard_eoas(&parsed, args.index, provider.as_ref()).await?)
    }
    .await;
    let (addresses, next_unused, scan_error) = match scan {
        Ok(scan) => (
            scan.addresses
                .iter()
                .map(|(index, address)| ScannedAddress {
                    index: *index,
                    address: address.to_string(),
                })
                .collect(),
            Some(scan.next_unused),
            None,
        ),
        Err(error) => (Vec::new(), None, Some(format!("{error:#}"))),
    };
    ImportReport {
        mnemonic_index: mnemonic.index,
        profile: ProfileSummary::from(&profile),
        addresses,
        next_unused,
        scan_error,
    }
    .emit(global.mode())
}

async fn add(global: &GlobalArgs, args: &AddArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let mnemonics = context.mnemonics().await?;
    if mnemonics.is_empty() {
        anyhow::bail!("no mnemonics; unlock a new network or run `edw profile generate`");
    }

    let mnemonic_index = if let Some(index) = args.mnemonic {
        resolve_mnemonic(&mnemonics, index)?.index
    } else if mnemonics.len() == 1 {
        mnemonics[0].index
    } else if global.non_interactive {
        anyhow::bail!(
            "--mnemonic is required when using --non-interactive and more than one mnemonic exists"
        );
    } else {
        select_mnemonic(&mnemonics)?
    };

    let profiles = context.profiles().await?;
    let profile_index = if args.next {
        next_profile_index(&profiles, mnemonic_index)
    } else {
        args.index
    };
    let name = prompt_profile_name(
        args.name.clone(),
        !global.non_interactive,
        &profiles,
        profile_index,
        None,
    )?;
    let record =
        bootstrap_profile(context.store.clone(), mnemonic_index, profile_index, name).await?;
    AddReport {
        profile: ProfileSummary::from(&record),
    }
    .emit(global.mode())
}

async fn rename(global: &GlobalArgs, args: &RenameArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let record = rename_profile(
        context.store.clone(),
        &args.selector,
        Some(args.new_name.clone()),
    )
    .await?;
    RenameReport {
        profile: ProfileSummary::from(&record),
    }
    .emit(global.mode())
}

fn select_mnemonic(mnemonics: &[mnemonic::MnemonicRecord]) -> Result<u32, anyhow::Error> {
    println!("Select a mnemonic:");
    for record in mnemonics {
        println!("  mnemonic {}", record.index);
    }
    if io::stdin().is_terminal() {
        print!("> ");
        io::stdout().flush()?;
    }
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let selector = line.trim();
    if selector.is_empty() {
        anyhow::bail!("no mnemonic selected");
    }
    let index = selector
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("mnemonic index must be a number"))?;
    Ok(resolve_mnemonic(mnemonics, index)?.index)
}

/// Resolves a new profile's name, prompting for it only when `interactive` and on a terminal.
pub fn prompt_profile_name(
    explicit: Option<String>,
    interactive: bool,
    profiles: &[ProfileRecord],
    profile_index: u32,
    except: Option<(u32, u32)>,
) -> Result<Option<String>, anyhow::Error> {
    let name = if let Some(name) = explicit {
        empty_name(name)
    } else if interactive && io::stdin().is_terminal() {
        print!("Profile name: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        None
    };

    let candidate = ProfileRecord {
        mnemonic_index: 0,
        profile_index,
        name: name.clone(),
    };
    let display = candidate.display_name();
    let taken = profiles.iter().any(|profile| {
        if except == Some((profile.mnemonic_index, profile.profile_index)) {
            return false;
        }
        profile.display_name() == display
    });
    if taken {
        anyhow::bail!("profile name `{display}` is already used");
    }
    Ok(name)
}

fn empty_name(name: String) -> Option<String> {
    if name.is_empty() || name == "-" {
        None
    } else {
        Some(name)
    }
}

fn read_phrase(non_interactive: bool) -> Result<Zeroizing<String>, anyhow::Error> {
    if io::stdin().is_terminal() {
        if non_interactive {
            anyhow::bail!(
                "--mnemonic-file or a phrase piped on stdin is required when using --non-interactive"
            );
        }
        print!("Mnemonic phrase: ");
        io::stdout().flush()?;
    }
    let mut phrase = Zeroizing::new(String::new());
    io::stdin().lock().read_line(&mut phrase)?;
    Ok(phrase)
}
