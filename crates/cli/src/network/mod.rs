use clap::Subcommand;

use crate::{
    GlobalArgs,
    network::add::{NetworkAddArgs, NetworkUseArgs},
};

pub mod add;
pub mod list;
pub mod status;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show this network's networkConfigs.
    View,
    /// Add a named networkConfig for this chain.
    Add(NetworkAddArgs),
    /// Use this networkConfig at runtime.
    Use(NetworkUseArgs),
    /// View network status, latest reported block-height, etc
    Status,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self {
            Command::View => list::run(global).await,
            Command::Add(args) => args.run(global).await,
            Command::Use(args) => args.run(global).await,
            Command::Status => {
                println!("Unimplemented");
                Ok(())
            }
        }
    }
}
