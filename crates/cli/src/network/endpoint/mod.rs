use std::fmt::Debug;

use anyhow::anyhow;
use clap::{Args, Subcommand};
use edw_core::network::{Network, db::NetworkDb};

use crate::{GlobalArgs, network::endpoint::list::NetworkEndpointListArgs};

mod list;

#[derive(Args, Debug)]
pub struct NetworkEndpointArgs {
    /// Network numeric ID or name.
    network: String,

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
        let context = global.gather().await?;
        let networks = context.networks.get_networks().await?;

        let network = networks
            .iter()
            .find(|x| x.network_id.0.to_string().eq(&self.network))
            .ok_or(anyhow!("network not found"))?;

        // TODO: subcommand should be able to throw errors but linter is not happy with it being pure right now
        self.command.run(network, global);
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
