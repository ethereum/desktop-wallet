use std::{clone::Clone, fmt::Debug};

use clap::Subcommand;

use crate::GlobalArgs;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Prints the resolved data directory.
    Path,
    /// Prints all resolved configuration values.
    View,
}

impl Command {
    pub fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Command::Path => {
                println!("{}", global.data_dir.display());
                Ok(())
            }
            Command::View => {
                println!("data_dir={}", global.data_dir.display());
                println!("network_store={}/network", global.data_dir.display());
                println!("profile_store={}/*/db", global.data_dir.display());
                match &global.rpc_url {
                    Some(rpc_url) => println!("rpc_url={rpc_url} (override)"),
                    None => println!("rpc_url=(from the active network)"),
                }
                Ok(())
            }
        }
    }
}
