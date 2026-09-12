use clap::{Args, Subcommand};
use edw_core::network::Network;

use crate::{GlobalArgs, network::endpoint::list::NetworkEndpointListArgs};

mod list;

#[derive(Args, Debug)]
pub struct NetworkEndpointArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Lists endpoints configured for a network.
    List(NetworkEndpointListArgs),
}

impl NetworkEndpointArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let network = global.gather().await?.preferences().await?;
        self.command.run(&network, global);
        Ok(())
    }
}

impl Command {
    pub fn run(&self, network: &Network, _global: &GlobalArgs) {
        match self {
            Self::List(args) => args.run(network),
        }
    }
}
