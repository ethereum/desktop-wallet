use clap::Subcommand;

use crate::{GlobalArgs, session::SessionFile};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Prints the resolved data directory.
    Path,
    /// Prints all resolved configuration values.
    View,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) {
        match self {
            Command::Path => {
                println!("{}", global.data_dir().path().display());
            }
            Command::View => {
                let data_dir = global.data_dir();
                println!("data_dir={}", data_dir.path().display());
                println!(
                    "network_store={}/<mainnet|sepolia|local|network id>",
                    data_dir.path().display()
                );
                match SessionFile::runtime().load().await {
                    Some(session) => println!("session={}", session.network),
                    None => println!("session=(locked)"),
                }
                match &global.rpc_url {
                    Some(rpc_url) => println!("rpc_url={rpc_url} (override)"),
                    None => println!("rpc_url=(from the unlocked network)"),
                }
            }
        }
    }
}
