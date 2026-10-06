use clap::{Parser, Subcommand};

use crate::{global_args::GlobalArgs, session::SessionFile};

mod asset;
mod balance;
mod config;
mod database;
mod global_args;
mod input;
mod network;
mod profile;
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
    /// Manages the network's assets and which profiles and accounts track them.
    #[command(subcommand)]
    Asset(asset::Command),
    /// Shows what profiles hold of their enabled assets.
    Balance(balance::BalanceArgs),
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
            Command::Config(args) => args.run(&self.global).await,
            Command::Profile(args) => args.run(&self.global).await?,
            Command::Asset(args) => args.run(&self.global).await?,
            Command::Balance(args) => args.run(&self.global).await?,
            Command::Database(args) => args.run(&self.global).await?,
            Command::Network(args) => args.run(&self.global).await?,
            Command::Unlock(args) => args.run(&self.global).await?,
            Command::Lock => {
                if SessionFile::runtime().clear().await? {
                    println!("Locked.");
                } else {
                    println!("Not unlocked; nothing to do.");
                }
            }
        }

        Ok(())
    }
}
