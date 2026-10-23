use std::fmt;

use clap::Args;
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Args, Debug)]
pub struct IndexerConfigArgs {
    /// Replace the indexer URL. Leaves on or off unchanged.
    #[arg(long)]
    url: Option<String>,
    /// Turn the indexer on. Requires a URL.
    #[arg(long, conflicts_with = "disable")]
    enable: bool,
    /// Turn the indexer off. Keeps the URL.
    #[arg(long)]
    disable: bool,
}

#[derive(Serialize)]
struct IndexerConfigReport {
    url: Option<String>,
    enabled: bool,
}

impl Report for IndexerConfigReport {
    const KIND: &'static str = "edw/network-indexer-config";
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

impl IndexerConfigArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let config = if let Some(url) = &self.url {
            instance.set_indexer_url(url).await?
        } else {
            instance.indexer_config().await?
        };
        let config = if self.enable {
            instance.set_indexer_enabled(true).await?
        } else if self.disable {
            instance.set_indexer_enabled(false).await?
        } else {
            config
        };
        IndexerConfigReport {
            url: config.url,
            enabled: config.enabled,
        }
        .emit(global.mode())
    }
}
