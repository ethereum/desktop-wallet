use clap::Subcommand;

use crate::{
    GlobalArgs,
    network::{add::NetworkAddArgs, endpoint::NetworkEndpointArgs, list::NetworkListArgs},
};

pub mod add;
pub mod endpoint;
pub mod list;
pub mod status;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Lists the configured networks
    List(NetworkListArgs),
    /// Add a new network
    Add(NetworkAddArgs),
    /// View network status, latest reported block-height, etc
    Status { id_or_preset: Option<String> },
    /// Manage network endpoints
    #[command(external_subcommand = false)]
    Endpoint(NetworkEndpointArgs),
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::List(args) => args.run(global).await,
            Command::Add(args) => args.run(global).await,
            Command::Endpoint(args) => args.run(global).await,
            Command::Status { id_or_preset: _ } => {
                println!("Unimplemented.");
                Ok(())
            }
        }
    }
}
