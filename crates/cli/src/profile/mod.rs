use std::sync::Arc;

use anyhow::Context as _;
use clap::Subcommand;
use edw_core::{
    executor::simple::SimpleExecutor,
    network::alloy::SimpleNetworkEndpoint,
    profile::simple::{SimpleProfile, db::SimpleProfileDb},
};

use crate::{GlobalArgs, context::Context};

#[derive(Subcommand)]
pub(crate) enum Command {
    /// List profiles.
    List,
    /// Create a profile with a random executor.
    Create { name: String },
    /// Show a profile's balance.
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
    let context = global.gather().await?;
    for name in context.profiles_index_db().list_profiles().await? {
        println!("{name}");
    }
    Ok(())
}

async fn create(name: &str, global: &GlobalArgs) -> Result<(), anyhow::Error> {
    if name.is_empty() {
        anyhow::bail!("profile name cannot be empty");
    }

    let context = global.gather().await?;
    let index = context.profiles_index_db();
    let mut names = index.list_profiles().await?;
    if names.iter().any(|existing| existing == name) {
        anyhow::bail!("profile `{name}` already exists");
    }

    let rpc_url = rpc_url(global, &context).await?;
    let provider = SimpleNetworkEndpoint::new_http(rpc_url.parse()?);
    let db: Arc<dyn edw_core::database::Database> = Arc::new(context.profile_db(name));

    SimpleProfile::new(provider, db, |ctx| async move {
        SimpleExecutor::new_with_random(ctx.provider, ctx.db).await
    })
    .await?;

    names.push(name.to_string());
    index.put_profiles(&names).await?;
    Ok(())
}

async fn rpc_url(global: &GlobalArgs, context: &Context) -> Result<String, anyhow::Error> {
    if let Some(url) = &global.rpc_url {
        return Ok(url.clone());
    }

    let (index, configs) = context.resolve_config(None).await?;
    configs[index]
        .http_rpc_url()
        .map(str::to_string)
        .context("no RPC endpoint; pass --rpc-url or run `edw network set-rpc <url>`")
}
