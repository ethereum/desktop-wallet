use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand};

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
    Config(config::ConfigArgs),
    /// Manages wallet profiles.
    Profile(profile::ProfileArgs),
    /// Manages profile databases.
    Database(database::DatabaseArgs),
    /// Manages configured networks and their endpoints.
    Network(network::NetworkArgs),
    /// Unlocks the wallet for this terminal session.
    Unlock,
    /// Locks the wallet, ending this terminal's session.
    Lock,
}

impl Cli {
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        match &self.command {
            Command::Config(args) => args.run(&self.global)?,
            Command::Profile(args) => args.run(&self.global).await?,
            Command::Database(args) => args.run(&self.global).await?,
            Command::Network(args) => args.run(&self.global).await?,
            Command::Unlock => unlock::run_unlock(&self.global).await?,
            Command::Lock => unlock::run_lock()?,
        }

        Ok(())
    }
}

pub(crate) fn print_help(name: &str) -> Result<(), anyhow::Error> {
    let mut command = Cli::command();
    let mut subcommand = &mut command;
    for part in name.split_whitespace() {
        subcommand = subcommand
            .find_subcommand_mut(part)
            .ok_or_else(|| anyhow::anyhow!("unknown command: {name}"))?;
    }
    subcommand.set_bin_name(format!("edw {name}"));
    subcommand.print_help()?;
    println!();
    Ok(())
}
