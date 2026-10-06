use clap::Subcommand;

use crate::{GlobalArgs, network::status::NetworkStatusArgs};

pub mod endpoint;
pub mod status;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show the unlocked network, its active endpoint, and the endpoint's latest block.
    Status(NetworkStatusArgs),
    /// Manage how this network is reached.
    #[command(subcommand)]
    Endpoint(endpoint::Command),
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::Status(args) => args.run(global).await,
            Command::Endpoint(command) => command.run(global).await,
        }
    }
}
