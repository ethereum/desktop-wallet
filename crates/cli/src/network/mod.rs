use clap::Subcommand;

use crate::{GlobalArgs, network::view::NetworkViewArgs};

pub mod rpc_config;
pub mod view;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show the unlocked network and its active RPC config.
    View(NetworkViewArgs),
    /// Manage how this network is reached.
    #[command(subcommand)]
    RpcConfig(rpc_config::Command),
    /// View network status, latest reported block-height, etc
    Status,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::View(args) => args.run(global).await,
            Command::RpcConfig(command) => command.run(global).await,
            Command::Status => anyhow::bail!("network status is not implemented"),
        }
    }
}
