use clap::{Args, Subcommand};
use edw_core::network::NetworkConfig;

use crate::{GlobalArgs, network::endpoint::list::NetworkEndpointListArgs};

mod list;

#[derive(Args, Debug)]
pub struct NetworkEndpointArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Lists endpoints configured for a networkConfig.
    List(NetworkEndpointListArgs),
}

impl NetworkEndpointArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let context = global.gather().await?;
        let name = match &self.command {
            Command::List(args) => args.name.as_deref(),
        };
        let (index, configs) = context.resolve_config(name).await?;
        self.command.run(&configs[index], global);
        Ok(())
    }
}

impl Command {
    pub fn run(&self, config: &NetworkConfig, _global: &GlobalArgs) {
        match self {
            Self::List(args) => args.run(config),
        }
    }
}
