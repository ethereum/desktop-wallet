use std::{
    fs::create_dir_all,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use clap::Subcommand;
use edw_core::{
    database::file::FileDatabase,
    executor::simple::SimpleExecutor,
    network::{alloy::SimpleNetworkEndpoint, presets::NetworkPreset},
    profile::simple::SimpleProfile,
};

use crate::GlobalArgs;

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Lists profile directories.
    List,
    /// Creates a new profile with a random executor.
    Create { name: String },
    /// Lists the balance of a profile.
    Balance { name: String },
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::List => list(global).await,
            Command::Create { name } => create(name, global).await,
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
        if entry.file_type().await?.is_dir() {
            println!("{}", entry.file_name().to_string_lossy());
        }
    }

    Ok(())
}

async fn create(name: &str, global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let profile_path = profile_path(name, &global.data_dir);
    if !profile_path.exists() {
        create_dir_all(&profile_path).context("error creating profile directory")?;
    }

    let rpc_url = global
        .rpc_url
        .as_deref()
        .or_else(|| NetworkPreset::LocalTestnet.default_rpc_url())
        .context("no RPC endpoint; pass --rpc-url")?;
    let provider = SimpleNetworkEndpoint::new_http(rpc_url.parse()?);
    let db = Arc::new(profile_db(name, &global.data_dir)?);

    SimpleProfile::new(provider, db, |ctx| async move {
        SimpleExecutor::new_with_random(ctx.provider, ctx.db).await
    })
    .await?;

    Ok(())
}

fn profile_db(name: &str, data_dir: &Path) -> Result<FileDatabase, anyhow::Error> {
    let db_path = profile_path(name, data_dir).join("db");
    FileDatabase::open(db_path).context("error opening profile database")
}

fn profile_path(name: &str, data_dir: &Path) -> PathBuf {
    data_dir.join(name)
}
