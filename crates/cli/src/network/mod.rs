use clap::Subcommand;

use crate::{
    GlobalArgs,
    network::{add::NetworkSetRpcArgs, endpoint::NetworkEndpointArgs},
};

pub mod add;
pub mod endpoint;
pub mod list;
pub mod status;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show this network's configuration.
    View,
    /// Set the HTTP RPC URL.
    SetRpc(NetworkSetRpcArgs),
    /// View network status, latest reported block-height, etc
    Status,
    /// Manage network endpoints
    #[command(external_subcommand = false)]
    Endpoint(NetworkEndpointArgs),
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::View => list::run(global).await,
            Command::SetRpc(args) => args.run(global).await,
            Command::Endpoint(args) => args.run(global).await,
            Command::Status => {
                println!("Unimplemented");
                Ok(())
            }
        }
    }
}
