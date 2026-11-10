use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::{
    failure::ErrorReport,
    global_args::GlobalArgs,
    report::{Mode, Report},
};

mod asset;
mod balance;
mod config;
mod failure;
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
    /// Configures the unlocked network instance.
    #[command(subcommand)]
    Network(network::Command),
    /// Lists the ERC-20s a profile tracks, and adds or removes them.
    #[command(subcommand)]
    Asset(asset::Command),
    /// Shows a profile's balance of each asset across its vaults.
    Balance(balance::BalanceArgs),
    /// Unlocks one network (default mainnet); any other network is locked.
    Unlock(unlock::UnlockArgs),
    /// Locks the wallet.
    Lock,
    /// Prints an address to receive funds on: the next unused one, stored unless `--peek` is set.
    Receive(fresh::ReceiveArgs),
    /// Deposits ETH or supported ERC-20 tokens into Tornado Cash.
    Shield(shield::ShieldArgs),
    /// Withdraws whole Tornado Cash notes of ETH or a supported ERC-20 to a fresh address.
    Unshield(unshield::UnshieldArgs),
    /// Sends ETH or a tracked ERC-20 from an account, or ETH from Tornado Cash.
    Transfer(transfer::TransferArgs),
}

impl Cli {
    /// The log level that `-v` and `-q` select, starting from `warn`.
    #[must_use]
    pub fn log_level(&self) -> tracing_subscriber::filter::LevelFilter {
        self.global.verbosity.tracing_level_filter()
    }

    /// Reports `error`, as an `edw/error` document on stdout under `--non-interactive` or as
    /// text on stderr otherwise, and returns the exit code for its class.
    #[must_use]
    pub fn fail(&self, error: &anyhow::Error) -> ExitCode {
        let report = ErrorReport::new(error);
        let mode = self.global.mode();
        if matches!(mode, Mode::Human) || report.emit(mode).is_err() {
            eprintln!("Error: {error:?}");
        }
        ExitCode::from(report.class().exit_code())
    }

    /// Runs the command, then, if it prompted for anything, prints the command line that carries
    /// each answer as a flag.
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        self.dispatch().await?;
        if let Some(command) = input::replay() {
            eprintln!("With your answers as flags: {command}");
        }
        Ok(())
    }

    async fn dispatch(&self) -> Result<(), anyhow::Error> {
        match &self.command {
            Command::Config(args) => args.run(&self.global).await,
            Command::Profile(args) => args.run(&self.global).await,
            Command::Network(args) => args.run(&self.global).await,
            Command::Asset(args) => args.run(&self.global).await,
            Command::Balance(args) => args.run(&self.global).await,
            Command::Unlock(args) => args.run(&self.global).await,
            Command::Lock => unlock::run_lock(&self.global).await,
            Command::Receive(args) => fresh::run(&self.global, args).await,
            Command::Shield(args) => shield::run(&self.global, args).await,
            Command::Unshield(args) => unshield::run(&self.global, args).await,
            Command::Transfer(args) => transfer::run(&self.global, args).await,
        }
    }
}
