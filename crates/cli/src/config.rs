use std::fmt;

use clap::Subcommand;
use serde::Serialize;

use crate::{GlobalArgs, report::Report, session::Session};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Prints the resolved data directory.
    Path,
    /// Prints all resolved configuration values.
    View,
}

#[derive(Serialize)]
struct ConfigPathReport {
    data_dir: String,
}

#[derive(Serialize)]
struct ConfigViewReport {
    data_dir: String,
    /// `None` while the wallet is locked.
    session: Option<String>,
    /// `None` when the unlocked network's endpoint applies.
    rpc_url: Option<String>,
}

impl Report for ConfigPathReport {
    const KIND: &'static str = "edw/config-path";
    const VERSION: u32 = 1;
}

impl fmt::Display for ConfigPathReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.data_dir)
    }
}

impl Report for ConfigViewReport {
    const KIND: &'static str = "edw/config-view";
    const VERSION: u32 = 1;
}

impl fmt::Display for ConfigViewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "data_dir={}", self.data_dir)?;
        writeln!(
            f,
            "network_store={}/{{mainnet|sepolia|local}}",
            self.data_dir
        )?;
        match &self.session {
            Some(network) => writeln!(f, "session={network}")?,
            None => writeln!(f, "session=(locked)")?,
        }
        match &self.rpc_url {
            Some(rpc_url) => write!(f, "rpc_url={rpc_url} (override)"),
            None => write!(f, "rpc_url=(from the unlocked network)"),
        }
    }
}

impl Command {
    pub fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let data_dir = global.data_dir.display().to_string();
        match self {
            Command::Path => ConfigPathReport { data_dir }.emit(global.mode()),
            Command::View => ConfigViewReport {
                data_dir,
                session: Session::load().map(|session| session.network.to_string()),
                rpc_url: global.rpc_url.clone(),
            }
            .emit(global.mode()),
        }
    }
}
