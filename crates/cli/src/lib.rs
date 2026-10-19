use clap::{Parser, Subcommand};

use crate::global_args::GlobalArgs;

mod balance;
mod config;
mod database;
mod fresh;
mod global_args;
mod input;
mod network;
mod profile;
mod report;
mod secret_file;
mod session;
mod shield;
mod spend;
mod transfer;
mod unlock;
mod unshield;

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
    /// Shows a profile's balance of each asset across its vaults.
    Balance(balance::BalanceArgs),
    /// Unlocks one network (default mainnet); any other network is locked.
    Unlock(unlock::UnlockArgs),
    /// Locks the wallet.
    Lock,
    /// Prints the next unused standard address, and stores it unless `--peek` is set.
    NextFreshAddress(fresh::NextFreshAddressArgs),
    /// Deposits native ETH into Tornado Cash.
    Shield(shield::ShieldArgs),
    /// Withdraws native ETH from Tornado Cash.
    Unshield(unshield::UnshieldArgs),
    /// Sends native ETH from an account, or from Tornado Cash.
    Transfer(transfer::TransferArgs),
}

impl Cli {
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        match &self.command {
            Command::Config(args) => args.run(&self.global).await,
            Command::Profile(args) => args.run(&self.global).await,
            Command::Database(args) => args.run(&self.global).await,
            Command::Network(args) => args.run(&self.global).await,
            Command::Balance(args) => args.run(&self.global).await,
            Command::Unlock(args) => args.run(&self.global).await,
            Command::Lock => unlock::run_lock(&self.global).await,
            Command::NextFreshAddress(args) => fresh::run(&self.global, args).await,
            Command::Shield(args) => shield::run(&self.global, args).await,
            Command::Unshield(args) => unshield::run(&self.global, args).await,
            Command::Transfer(args) => transfer::run(&self.global, args).await,
        }
    }
}
