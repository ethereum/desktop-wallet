use std::io::{self, BufRead, IsTerminal, Write};

use clap::{Args, Subcommand};
use zeroize::Zeroizing;

use crate::GlobalArgs;

#[derive(Subcommand)]
pub enum Command {
    /// List profiles grouped by mnemonic.
    List(ListArgs),
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
pub struct ListArgs {}

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

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List(args) => args.run(global).await,
            Self::Generate(args) => args.run(global).await,
            Self::Import(args) => args.run(global).await,
            Self::Add(args) => args.run(global).await,
            Self::Rename(args) => args.run(global).await,
        }
    }
}

impl ListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let mut profiles = global.open().await?.profiles().await?;
        if profiles.is_empty() {
            println!(
                "No profiles. Create one with `edw profile generate` or `edw profile import`."
            );
            return Ok(());
        }

        profiles.sort_by_key(|profile| (profile.mnemonic_index, profile.profile_index));
        let mut mnemonic = None;
        for profile in &profiles {
            if mnemonic != Some(profile.mnemonic_index) {
                mnemonic = Some(profile.mnemonic_index);
                println!("Mnemonic {}", profile.mnemonic_index);
            }
            println!("  {}  {}", profile.profile_index, profile.display_name());
        }
        Ok(())
    }
}

impl GenerateArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let name = prompt_profile_name(self.name.clone())?;
        let (mnemonic, profile) = instance
            .generate_profile(self.long_seed, self.index, name)
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
}

impl ImportArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let phrase = read_phrase()?;
        let name = prompt_profile_name(self.name.clone())?;
        let (mnemonic, profile) = instance.import_profile(phrase, self.index, name).await?;
        println!(
            "Imported mnemonic {}; profile {}/{} ({}).",
            mnemonic.index,
            profile.mnemonic_index,
            profile.profile_index,
            profile.display_name()
        );

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

impl AddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let indices = instance.mnemonic_indices().await?;
        let mnemonic_index = match (self.mnemonic, indices.as_slice()) {
            (_, []) => {
                anyhow::bail!("no mnemonics; run `edw profile generate` or `edw profile import`")
            }
            (Some(index), _) => index,
            (None, [only]) => *only,
            (None, _) => select_mnemonic(&indices)?,
        };
        let profile_index = (!self.next).then_some(self.index);
        let name = prompt_profile_name(self.name.clone())?;
        let record = instance
            .add_profile(mnemonic_index, profile_index, name)
            .await?;
        println!(
            "Created profile {}/{} ({}).",
            record.mnemonic_index,
            record.profile_index,
            record.display_name()
        );
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
        println!(
            "Renamed profile {}/{} to {}.",
            record.mnemonic_index,
            record.profile_index,
            record.display_name()
        );
        Ok(())
    }
}

/// `explicit` when given, else a name typed at the terminal. Validation is the instance's.
fn prompt_profile_name(explicit: Option<String>) -> Result<Option<String>, anyhow::Error> {
    if explicit.is_some() || !io::stdin().is_terminal() {
        return Ok(explicit);
    }
    print!("Profile name: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}

// TODO: maybe replace or relocate: one of several stdin prompt helpers.
fn select_mnemonic(indices: &[u32]) -> Result<u32, anyhow::Error> {
    println!("Select a mnemonic:");
    for index in indices {
        println!("  mnemonic {index}");
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
    selector
        .parse()
        .map_err(|_| anyhow::anyhow!("mnemonic index must be a number"))
}

// TODO: maybe replace or relocate: one of several stdin prompt helpers.
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
