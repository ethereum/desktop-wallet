use std::{
    fmt,
    io::{self, BufRead, IsTerminal, Read, Write},
    path::PathBuf,
};

use clap::{Args, Subcommand};
use edw_core::{
    mnemonic::{self, resolve_mnemonic, scan::scan_standard_eoas},
    profile::simple::{
        ProfileRecord, bootstrap_profile, db::SimpleProfileDb, next_profile_index, rename_profile,
        resolve_profile,
    },
};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{GlobalArgs, report::Report, secret_file, session, unlock};

const MNEMONIC_MAX_BYTES: u64 = 4096;

pub const PHRASE_WITHHELD: &str =
    "The recovery phrase was not shown. Run `edw profile reveal-seed` at a terminal to back it up.";

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
    /// Show the recovery phrase behind a profile. Runs only at a terminal.
    RevealSeed(RevealSeedArgs),
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
    /// Mnemonic index. Prompted for when more than one mnemonic exists; required then with
    /// --non-interactive.
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

#[derive(Args, Debug)]
pub struct RevealSeedArgs {
    /// Profile as `mnemonic/profile` or a unique name. Prompted for when more than one exists.
    selector: Option<String>,
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
    next_unused: Option<u32>,
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
            Self::RevealSeed(args) => reveal_seed(global, args).await,
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
    if global.non_interactive {
        eprintln!("{PHRASE_WITHHELD}");
    } else {
        println!("Write this recovery phrase down now.");
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
        Some(path) => secret_file::read(path, "mnemonic", MNEMONIC_MAX_BYTES)?,
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
    // The error can name the RPC URL, which may carry an API key, so it stays off stdout.
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
            eprintln!("The address scan failed: {error:#}");
            (Vec::new(), None)
        }
    };
    ImportReport {
        mnemonic_index: mnemonic.index,
        profile: ProfileSummary::from(&profile),
        addresses,
        next_unused,
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

async fn reveal_seed(global: &GlobalArgs, args: &RevealSeedArgs) -> Result<(), anyhow::Error> {
    if global.non_interactive {
        anyhow::bail!("profile reveal-seed never prints the phrase with --non-interactive");
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("profile reveal-seed only runs at a terminal");
    }

    let context = global.gather().await?;
    // An unlocked session alone must not be enough to read the seed.
    let password = unlock::prompt("Decryption password: ")?;
    unlock::open_existing_store(
        &session::canonical_data_dir(&global.data_dir),
        context.network,
        password.as_bytes(),
    )
    .await?;

    let profiles = context.profiles().await?;
    let profile = match (&args.selector, profiles.as_slice()) {
        (Some(selector), _) => resolve_profile(&profiles, selector)?,
        (None, []) => anyhow::bail!("no profiles"),
        (None, [only]) => only,
        (None, _) => select_profile(&profiles)?,
    };
    let mnemonics = context.mnemonics().await?;
    let mnemonic = resolve_mnemonic(&mnemonics, profile.mnemonic_index)?;

    println!(
        "Anyone who sees this phrase can take the funds of every profile on mnemonic {}.",
        mnemonic.index
    );
    if !confirm(&format!(
        "Show the recovery phrase for profile {}?",
        ProfileSummary::from(profile)
    ))? {
        println!("Not shown.");
        return Ok(());
    }
    println!();
    println!("{}", mnemonic.phrase);
    println!();
    println!("Store it offline, then clear your terminal scrollback.");
    Ok(())
}

fn select_profile(profiles: &[ProfileRecord]) -> Result<&ProfileRecord, anyhow::Error> {
    println!("Select a profile:");
    for profile in profiles {
        println!("  {}", ProfileSummary::from(profile));
    }
    print!("> ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(resolve_profile(profiles, line.trim())?)
}

fn confirm(question: &str) -> Result<bool, anyhow::Error> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
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
        let mut phrase = Zeroizing::new(String::new());
        io::stdin().lock().read_line(&mut phrase)?;
        return Ok(phrase);
    }

    let mut phrase = Zeroizing::new(String::new());
    io::stdin()
        .lock()
        .take(MNEMONIC_MAX_BYTES + 1)
        .read_to_string(&mut phrase)?;
    if phrase.len() as u64 > MNEMONIC_MAX_BYTES {
        anyhow::bail!(
            "the mnemonic phrase on stdin is too large (maximum {MNEMONIC_MAX_BYTES} bytes)"
        );
    }
    Ok(phrase)
}
