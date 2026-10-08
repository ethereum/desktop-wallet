use clap::Subcommand;

use crate::{GlobalArgs, network::view::NetworkViewArgs};

pub mod endpoint;
pub mod view;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show the unlocked network and its active endpoint.
    View(NetworkViewArgs),
    /// Manage how this network is reached.
    #[command(subcommand)]
    Endpoint(endpoint::Command),
    /// View network status, latest reported block-height, etc
    Status,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::View(args) => args.run(global).await,
            Command::Endpoint(command) => command.run(global).await,
            Command::Status => {
                println!("Unimplemented");
                Ok(())
            }
        }
    }
}
