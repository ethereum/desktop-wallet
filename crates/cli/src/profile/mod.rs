use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Subcommand;
use edw_core::{
    network::{NetworkId, SimpleNetworkEndpoint, db::NetworkDb, presets::NetworkPreset},
    seed::{Mnemonic, SeedRecord, WordCount, assert_network, scan_used, store_seed},
};
use zeroize::Zeroizing;

use crate::{GlobalArgs, unlock};

const NETWORK_SUBDIR: &str = "network";

/// Data-dir folders that are not profiles and cannot be used as profile names.
const RESERVED_DATA_SUBDIRS: &[&str] = &[NETWORK_SUBDIR];

fn is_reserved_data_subdir(name: &str) -> bool {
    RESERVED_DATA_SUBDIRS
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Lists profile directories.
    List,
    /// Creates a profile from a new BIP-39 seed.
    Create {
        name: String,
        /// Preset slug or chain id (`sepolia`, `local`, `1`, …).
        #[arg(long)]
        network: String,
        /// BIP-44 account' / profile index. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        profile_index: u32,
        /// Generate a 24-word mnemonic instead of 12.
        #[arg(long)]
        long_seed: bool,
    },
    /// Restores a profile from an existing mnemonic and scans used addresses.
    Import {
        name: String,
        /// Preset slug or chain id (`sepolia`, `local`, `1`, …).
        #[arg(long)]
        network: String,
        /// BIP-44 account' / profile index. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        profile_index: u32,
        /// Mnemonic phrase (otherwise prompted).
        #[arg(long)]
        mnemonic: Option<String>,
    },
    /// Lists the balance of a profile.
    Balance { name: String },
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::List => list(global).await,
            Command::Create {
                name,
                network,
                profile_index,
                long_seed,
            } => create(name, network, *profile_index, *long_seed, global).await,
            Command::Import {
                name,
                network,
                profile_index,
                mnemonic,
            } => import(name, network, *profile_index, mnemonic.as_deref(), global).await,
            Command::Balance { name } => {
                println!("Balance lookup for profile `{name}` is not implemented");
                Ok(())
            }
        }
    }
}

async fn list(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let mut entries = match tokio::fs::read_dir(&global.data_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_reserved_data_subdir(&name) {
            continue;
        }
        println!("{name}");
    }

    Ok(())
}

async fn create(
    name: &str,
    network: &str,
    profile_index: u32,
    long_seed: bool,
    global: &GlobalArgs,
) -> Result<(), anyhow::Error> {
    assert_usable_profile_name(name)?;
    let network_id = resolve_network_id(network, global).await?;
    let path = profile_path(name, &global.data_dir);
    if path.exists() {
        anyhow::bail!("a profile named `{name}` already exists");
    }

    let word_count = if long_seed {
        WordCount::TwentyFour
    } else {
        WordCount::Twelve
    };
    let record = SeedRecord::new(Mnemonic::generate(word_count)?, network_id, profile_index);

    std::fs::create_dir_all(&path).context("error creating profile directory")?;
    if let Err(error) = async {
        let db = unlock::profile_store(name, &global.data_dir).await?;
        store_seed(db.as_ref(), &record).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await
    {
        let _ = std::fs::remove_dir_all(&path);
        return Err(error);
    }

    println!("Write these words down. They are shown once.");
    println!();
    println!("{}", record.words());
    println!();
    println!("Anyone with these words can take the funds in this profile.");
    Ok(())
}

async fn import(
    name: &str,
    network: &str,
    profile_index: u32,
    mnemonic_flag: Option<&str>,
    global: &GlobalArgs,
) -> Result<(), anyhow::Error> {
    assert_usable_profile_name(name)?;
    let network_id = resolve_network_id(network, global).await?;
    let path = profile_path(name, &global.data_dir);
    if path.exists() {
        anyhow::bail!("a profile named `{name}` already exists");
    }

    let phrase = match mnemonic_flag {
        Some(phrase) => Zeroizing::new(phrase.to_owned()),
        None => unlock::prompt("Mnemonic: ")?,
    };
    let record = SeedRecord::new(Mnemonic::parse(&phrase)?, network_id, profile_index);
    let provider = rpc_provider(global, network_id)?;
    assert_network(&record, &provider).await?;

    std::fs::create_dir_all(&path).context("error creating profile directory")?;
    let result = async {
        let db = unlock::profile_store(name, &global.data_dir).await?;
        store_seed(db.as_ref(), &record).await?;
        let scan = scan_used(&record, &provider).await?;
        Ok::<_, anyhow::Error>(scan)
    }
    .await;

    let scan = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&path);
            return Err(error);
        }
    };

    println!(
        "Imported profile `{name}` on chain {} (profile index {}).",
        network_id.0, profile_index
    );
    println!("nextIndex={}", scan.next_index);
    for (index, address) in &scan.used {
        println!("{index}  {address}");
    }
    Ok(())
}

async fn resolve_network_id(input: &str, global: &GlobalArgs) -> Result<NetworkId, anyhow::Error> {
    if let Some(preset) = NetworkPreset::from_input(input) {
        return Ok(preset.network_id());
    }

    let store = unlock::try_network_store(&global.data_dir).await?;
    let Some(store) = store else {
        anyhow::bail!("unknown network `{input}`");
    };
    let networks = store.get_networks().await?;
    networks
        .iter()
        .find(|network| {
            network.name.eq_ignore_ascii_case(input) || network.network_id.0.to_string() == input
        })
        .map(|network| network.network_id)
        .ok_or_else(|| anyhow::anyhow!("unknown network `{input}`"))
}

fn rpc_provider(
    global: &GlobalArgs,
    network_id: NetworkId,
) -> Result<SimpleNetworkEndpoint, anyhow::Error> {
    let url = if let Some(url) = &global.rpc_url {
        url.clone()
    } else if network_id == NetworkPreset::LocalTestnet.network_id() {
        NetworkPreset::LocalTestnet
            .default_rpc_url()
            .context("local testnet has no default RPC")?
            .to_owned()
    } else {
        anyhow::bail!("no RPC endpoint; pass --rpc-url");
    };
    Ok(SimpleNetworkEndpoint::new_http(url.parse()?))
}

fn assert_usable_profile_name(name: &str) -> Result<(), anyhow::Error> {
    if is_reserved_data_subdir(name) {
        anyhow::bail!("`{name}` is reserved and cannot be used as a profile name");
    }
    Ok(())
}

fn profile_path(name: &str, data_dir: &Path) -> PathBuf {
    data_dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_is_a_reserved_data_subdir() {
        assert!(is_reserved_data_subdir("network"));
        assert!(is_reserved_data_subdir("NETWORK"));
        assert!(is_reserved_data_subdir("NeTwOrK"));
        assert!(!is_reserved_data_subdir("alice"));
        assert_eq!(RESERVED_DATA_SUBDIRS, &[NETWORK_SUBDIR]);
    }
}
