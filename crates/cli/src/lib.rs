use clap::{Parser, Subcommand};

use crate::context::GlobalArgs;

mod config;
mod context;
mod database;
mod network;
mod profile;
mod report;
mod secret_file;
mod session;
mod unlock;

#[derive(Parser)]
#[command(name = "edw", about = "Ethereum Desktop Wallet CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    global: GlobalArgs,
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
    /// Configures the unlocked network instance.
    #[command(subcommand)]
    Network(network::Command),
    /// Unlocks one network (default mainnet); any other network is locked.
    Unlock(unlock::UnlockArgs),
    /// Locks the wallet.
    Lock,
}

impl Cli {
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        match &self.command {
            Command::Config(args) => args.run(&self.global)?,
            Command::Profile(args) => args.run(&self.global).await?,
            Command::Database(args) => args.run(&self.global).await?,
            Command::Network(args) => args.run(&self.global).await?,
            Command::Unlock(args) => args.run(&self.global).await?,
            Command::Lock => unlock::run_lock(&self.global)?,
        }

        Ok(())
    }
}
