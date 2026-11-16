use clap::Subcommand;

use crate::{GlobalArgs, network::view::NetworkViewArgs};

pub mod bundler;
pub mod indexer;
pub mod rpc;
pub mod view;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show the unlocked network and the HTTP services it calls.
    View(NetworkViewArgs),
    /// Manage how this network is reached.
    #[command(subcommand)]
    Rpc(rpc::Command),
    /// Bundler URLs, tried in order. Each later URL is a fallback.
    #[command(subcommand)]
    Bundler(bundler::Command),
    /// The pool indexer: its URL, and whether it is on. Shows it without a subcommand.
    Indexer(indexer::IndexerArgs),
    /// View network status, latest reported block-height, etc
    #[command(hide = true)]
    Status,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::View(args) => args.run(global).await,
            Command::Rpc(command) => command.run(global).await,
            Command::Bundler(command) => command.run(global).await,
            Command::Indexer(args) => args.run(global).await,
            Command::Status => anyhow::bail!("network status is not implemented"),
        }
    }
}
