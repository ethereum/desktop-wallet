use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

mod config;
mod context;
mod database;
mod network;
mod output;
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

#[derive(Args)]
pub(crate) struct GlobalArgs {
    #[arg(long, global = true, env = "DATA_DIR", default_value = "./.edw/")]
    pub(crate) data_dir: PathBuf,
    /// Overrides the unlocked network's endpoint for this invocation.
    #[arg(long, global = true, env = "RPC_URL")]
    pub(crate) rpc_url: Option<String>,
    /// Emit one JSON document on stdout, never prompt, and fail if an input is missing.
    #[arg(long, global = true)]
    pub(crate) non_interactive: bool,
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

impl GlobalArgs {
    pub(crate) fn mode(&self) -> output::Mode {
        if self.non_interactive {
            output::Mode::Json
        } else {
            output::Mode::Human
        }
    }
}

impl Command {
    /// Whether this command emits the single JSON document `--non-interactive` promises.
    fn emits_json(&self) -> bool {
        matches!(
            self,
            Self::Unlock(_) | Self::Network(network::Command::View)
        )
    }
}

impl Cli {
    pub async fn run(&self) -> Result<(), anyhow::Error> {
        if self.global.non_interactive && !self.command.emits_json() {
            anyhow::bail!(
                "this command has no --non-interactive output yet; run it without the flag"
            );
        }

        match &self.command {
            Command::Config(args) => args.run(&self.global),
            Command::Profile(args) => args.run(&self.global).await?,
            Command::Database(args) => args.run(&self.global).await?,
            Command::Network(args) => args.run(&self.global).await?,
            Command::Unlock(args) => unlock::run_unlock(&self.global, args).await?,
            Command::Lock => unlock::run_lock()?,
        }

        Ok(())
    }
}
