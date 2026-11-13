use std::fmt;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Args, Debug)]
pub struct IndexerArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Replace the indexer URL. Leaves on or off unchanged.
    SetUrl {
        /// http or https indexer URL.
        url: String,
    },
    /// Turn the indexer on. Requires a URL.
    Enable,
    /// Turn the indexer off. Keeps the URL.
    Disable,
}

#[derive(Serialize)]
struct IndexerConfigReport {
    url: Option<String>,
    enabled: bool,
}

impl Report for IndexerConfigReport {
    const KIND: &'static str = "edw/network-indexer";
    const VERSION: u32 = 1;
}

impl fmt::Display for IndexerConfigReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.url, self.enabled) {
            (Some(url), true) => write!(f, "indexer {url} on"),
            (Some(url), false) => write!(f, "indexer {url} off"),
            (None, _) => write!(f, "indexer off"),
        }
    }
}

impl IndexerArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let config = match &self.command {
            None => instance.indexer_config().await?,
            Some(Command::SetUrl { url }) => instance.set_indexer_url(url).await?,
            Some(Command::Enable) => instance.set_indexer_enabled(true).await?,
            Some(Command::Disable) => instance.set_indexer_enabled(false).await?,
        };
        IndexerConfigReport {
            url: config.url,
            enabled: config.enabled,
        }
        .emit(global.mode())
    }
}
