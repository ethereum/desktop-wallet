use std::fmt;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List bundler URLs in the order they are tried.
    List(BundlerConfigListArgs),
    /// Append a fallback bundler URL.
    Add(BundlerConfigAddArgs),
    /// Remove a bundler URL. The last one cannot be removed.
    Remove(BundlerConfigRemoveArgs),
}

#[derive(Args, Debug)]
pub struct BundlerConfigListArgs {}

#[derive(Args, Debug)]
pub struct BundlerConfigAddArgs {
    /// http or https bundler URL.
    #[arg(long)]
    url: String,
}

#[derive(Args, Debug)]
pub struct BundlerConfigRemoveArgs {
    /// http or https bundler URL.
    #[arg(long)]
    url: String,
}

#[derive(Serialize)]
struct BundlerConfigReport {
    urls: Vec<String>,
}

impl Report for BundlerConfigReport {
    const KIND: &'static str = "edw/network-bundler-config";
    const VERSION: u32 = 1;
}

impl fmt::Display for BundlerConfigReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.urls.is_empty() {
            return write!(
                f,
                "No bundler URLs. Add one with `edw network bundler-config add --url <url>`."
            );
        }
        write!(f, "{}", self.urls.join("\n"))
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List(args) => args.run(global).await,
            Self::Add(args) => args.run(global).await,
            Self::Remove(args) => args.run(global).await,
        }
    }
}

impl BundlerConfigListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let urls = global.open().await?.bundler_urls().await?;
        BundlerConfigReport {
            urls: urls.iter().map(ToString::to_string).collect(),
        }
        .emit(global.mode())
    }
}

impl BundlerConfigAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let urls = global.open().await?.add_bundler_url(&self.url).await?;
        BundlerConfigReport {
            urls: urls.iter().map(ToString::to_string).collect(),
        }
        .emit(global.mode())
    }
}

impl BundlerConfigRemoveArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let urls = global.open().await?.remove_bundler_url(&self.url).await?;
        BundlerConfigReport {
            urls: urls.iter().map(ToString::to_string).collect(),
        }
        .emit(global.mode())
    }
}
