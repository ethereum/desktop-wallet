use clap::Subcommand;

use crate::{GlobalArgs, session};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Prints the resolved data directory.
    Path,
    /// Prints all resolved configuration values.
    View,
}

impl Command {
    pub fn run(&self, global: &GlobalArgs) {
        match self {
            Command::Path => {
                println!("{}", global.data_dir.display());
            }
            Command::View => {
                println!("data_dir={}", global.data_dir.display());
                println!(
                    "network_store={}/{{mainnet|sepolia|local}}",
                    global.data_dir.display()
                );
                match session::load() {
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
