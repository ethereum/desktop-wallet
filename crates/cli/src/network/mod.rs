use clap::{Args, Subcommand};

use crate::{
    GlobalArgs,
    network::{add::NetworkAddArgs, endpoint::NetworkEndpointArgs, list::NetworkListArgs},
    print_help,
};

pub mod add;
pub mod endpoint;
pub mod list;
pub mod status;

#[derive(Args)]
pub struct NetworkArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Lists the configured networks
    List(NetworkListArgs),
    /// Add a new network
    Add(NetworkAddArgs),
    /// View network status, latest reported block-height, etc
    Status {
        id_or_preset: Option<String>,
        /// Prints endpoint URLs in full
        #[arg(long)]
        show_urls: bool,
    },
    /// Manage network endpoints
    #[command(external_subcommand = false)]
    Endpoint(NetworkEndpointArgs),
}

impl NetworkArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match &self.command {
            None => print_help("network"),
            Some(Command::List(args)) => args.run(global).await,
            Some(Command::Add(args)) => args.run(global).await,
            // Some(Command::Status {
            //     id_or_preset,
            //     show_urls,
            // }) => status(id_or_preset.as_deref(), *show_urls, global).await,
            Some(Command::Endpoint(args)) => args.run(global).await,
            _ => {
                println!("Unimplemented.");
                Ok(())
            }
        }
    }
}
