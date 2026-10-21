use clap::Subcommand;

use crate::{GlobalArgs, network::view::NetworkViewArgs};

pub mod bundler_config;
pub mod indexer_config;
pub mod rpc_config;
pub mod view;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show the unlocked network and the HTTP services it calls.
    View(NetworkViewArgs),
    /// Manage how this network is reached.
    #[command(subcommand)]
    RpcConfig(rpc_config::Command),
    /// Bundler URLs, tried in order. Each later URL is a fallback.
    #[command(subcommand)]
    BundlerConfig(bundler_config::Command),
    /// The pool indexer: its URL, and whether it is on.
    IndexerConfig(indexer_config::IndexerConfigArgs),
    /// View network status, latest reported block-height, etc
    Status,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::View(args) => args.run(global).await,
            Command::RpcConfig(command) => command.run(global).await,
            Command::BundlerConfig(command) => command.run(global).await,
            Command::IndexerConfig(args) => args.run(global).await,
            Command::Status => anyhow::bail!("network status is not implemented"),
        }
    }
}
