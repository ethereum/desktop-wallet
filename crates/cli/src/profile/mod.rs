use std::io::{self, BufRead, IsTerminal, Write};

use clap::{Args, Subcommand};
use edw_core::{
    mnemonic::{self, resolve_mnemonic, scan::scan_standard_eoas},
    profile::simple::{
        ProfileRecord, bootstrap_profile, db::SimpleProfileDb, next_profile_index, rename_profile,
    },
};
use zeroize::Zeroizing;

use crate::GlobalArgs;

#[derive(Subcommand)]
pub(crate) enum Command {
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
pub(crate) struct GenerateArgs {
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
pub(crate) struct ImportArgs {
    #[arg(long)]
    name: Option<String>,
    /// Profile index to create. Defaults to 0.
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Args, Debug)]
pub(crate) struct AddArgs {
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
pub(crate) struct RenameArgs {
    /// Profile as `mnemonic/profile` or a unique name.
    selector: String,
    /// New name. Pass `-` or an empty string to clear it.
    new_name: String,
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
    if profiles.is_empty() {
        println!("No profiles.");
        return Ok(());
    }

    for mnemonic in &mnemonics {
        let mut group: Vec<_> = profiles
            .iter()
            .filter(|profile| profile.mnemonic_index == mnemonic.index)
            .collect();
        if group.is_empty() {
            continue;
        }
        group.sort_by_key(|profile| profile.profile_index);
        println!("Mnemonic {}", mnemonic.index);
        for profile in group {
            let _pointer = context
                .profile_db(profile.mnemonic_index, profile.profile_index)
                .get_pointer()
                .await?;
            println!("  {}  {}", profile.profile_index, profile.display_name());
        }
    }
    Ok(())
}

async fn generate(global: &GlobalArgs, args: &GenerateArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let profiles = context.profiles().await?;
    let name = prompt_profile_name(args.name.clone(), &profiles, args.index, None)?;
    let (mnemonic, profile) =
        mnemonic::generate_as_profile(context.store.clone(), args.long_seed, args.index, name)
            .await?;
    println!("Write this recovery phrase down now. It is shown only this once.");
    println!();
    println!("{}", mnemonic.phrase);
    println!();
    println!(
        "Created mnemonic {} and profile {}/{} ({}).",
        mnemonic.index,
        profile.mnemonic_index,
        profile.profile_index,
        profile.display_name()
    );
    Ok(())
}

async fn import(global: &GlobalArgs, args: &ImportArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let phrase = read_phrase()?;
    let profiles = context.profiles().await?;
    let name = prompt_profile_name(args.name.clone(), &profiles, args.index, None)?;
    let (mnemonic, profile) =
        mnemonic::import_as_profile(context.store.clone(), phrase, args.index, name).await?;
    println!(
        "Imported mnemonic {}; profile {}/{} ({}).",
        mnemonic.index,
        profile.mnemonic_index,
        profile.profile_index,
        profile.display_name()
    );

    let provider = context.endpoint(global.rpc_url.as_deref()).await?;
    let parsed = mnemonic.mnemonic()?;
    let scan = scan_standard_eoas(&parsed, args.index, &provider).await?;
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
    } else {
        select_mnemonic(&mnemonics)?
    };

    let profiles = context.profiles().await?;
    let profile_index = if args.next {
        next_profile_index(&profiles, mnemonic_index)
    } else {
        args.index
    };
    let name = prompt_profile_name(args.name.clone(), &profiles, profile_index, None)?;
    let record =
        bootstrap_profile(context.store.clone(), mnemonic_index, profile_index, name).await?;
    println!(
        "Created profile {}/{} ({}).",
        record.mnemonic_index,
        record.profile_index,
        record.display_name()
    );
    Ok(())
}

async fn rename(global: &GlobalArgs, args: &RenameArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let record = rename_profile(
        context.store.clone(),
        &args.selector,
        Some(args.new_name.clone()),
    )
    .await?;
    println!(
        "Renamed profile {}/{} to {}.",
        record.mnemonic_index,
        record.profile_index,
        record.display_name()
    );
    Ok(())
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

pub(crate) fn prompt_profile_name(
    explicit: Option<String>,
    profiles: &[ProfileRecord],
    profile_index: u32,
    except: Option<(u32, u32)>,
) -> Result<Option<String>, anyhow::Error> {
    let name = if let Some(name) = explicit {
        empty_name(name)
    } else if io::stdin().is_terminal() {
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

fn read_phrase() -> Result<Zeroizing<String>, anyhow::Error> {
    if io::stdin().is_terminal() {
        print!("Mnemonic phrase: ");
        io::stdout().flush()?;
    }
    let mut phrase = Zeroizing::new(String::new());
    io::stdin().lock().read_line(&mut phrase)?;
    Ok(Zeroizing::new(
        phrase.split_whitespace().collect::<Vec<_>>().join(" "),
    ))
}
