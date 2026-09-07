use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

mod config;
mod context;
mod database;
mod network;
mod profile;
mod session;
mod unlock;
mod utils;

#[derive(Parser)]
#[command(name = "edw", about = "Ethereum Desktop Wallet CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    global: GlobalArgs,
}

#[derive(Args)]
pub(crate) struct GlobalArgs {
    #[arg(long, global = true, env = "DATA_DIR", default_value = "./.edw/")]
    pub(crate) data_dir: PathBuf,
    /// Overrides the active network's endpoint for this invocation.
    #[arg(long, global = true, env = "RPC_URL")]
    pub(crate) rpc_url: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Inspects the resolved CLI configuration.
    #[command(subcommand)]
    Config(config::Command),
    /// Manages wallet profiles.
    #[command(subcommand)]
    Profile(profile::Command),
    /// Manages profile databases.
    #[command(subcommand)]
    Database(database::Command),
    /// Manages configured networks and their endpoints.
    #[command(subcommand)]
    Network(network::Command),
    /// Unlocks the wallet for this terminal session.
    Unlock,
    /// Locks the wallet, ending this terminal's session.
    Lock,
}

impl Cli {
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        match &self.command {
            Command::Config(args) => args.run(&self.global),
            Command::Profile(args) => args.run(&self.global).await?,
            Command::Database(args) => args.run(&self.global).await?,
            Command::Network(args) => args.run(&self.global).await?,
            Command::Unlock => unlock::run_unlock(&self.global).await?,
            Command::Lock => unlock::run_lock()?,
        }

        Ok(())
    }
}
