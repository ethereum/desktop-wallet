use std::{fmt, num::NonZeroU64};

use clap::{Args, Subcommand};
use edw_core::network::{DEFAULT_EVENT_BLOCK_RANGE, RpcConfig, RpcConfigKind};
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List this network's RPC configs. The active one is marked `*`.
    List(RpcConfigListArgs),
    /// Add an RPC config. It becomes active when none is, or when `--use` is set.
    Add(RpcConfigAddArgs),
    /// Make an RPC config the active one.
    Use(RpcConfigUseArgs),
}

#[derive(Args, Debug)]
pub struct RpcConfigListArgs {}

#[derive(Args, Debug)]
pub struct RpcConfigAddArgs {
    name: String,
    /// http or https JSON-RPC URL.
    #[arg(long)]
    url: String,
    /// The widest block span one log request may ask this RPC for.
    #[arg(long, default_value_t = DEFAULT_EVENT_BLOCK_RANGE)]
    event_block_range: NonZeroU64,
    /// Make this the active RPC config.
    #[arg(long)]
    r#use: bool,
}

#[derive(Args, Debug)]
pub struct RpcConfigUseArgs {
    name: String,
}

#[derive(Serialize)]
struct RpcConfigSummary {
    name: String,
    kind: &'static str,
    url: String,
    active: bool,
}

#[derive(Serialize)]
struct RpcConfigListReport {
    rpc_configs: Vec<RpcConfigSummary>,
}

#[derive(Serialize)]
struct RpcConfigAddReport {
    rpc_config: RpcConfigSummary,
}

#[derive(Serialize)]
struct RpcConfigUseReport {
    name: String,
}

impl RpcConfigSummary {
    fn new(config: RpcConfig, active: bool) -> Self {
        let (kind, url) = match config.kind {
            RpcConfigKind::Http { url } => ("http", url),
        };
        Self {
            name: config.name,
            kind,
            url,
            active,
        }
    }
}

impl fmt::Display for RpcConfigSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.name, self.kind, self.url)
    }
}

impl Report for RpcConfigListReport {
    const KIND: &'static str = "edw/network-rpc-config-list";
    const VERSION: u32 = 1;
}

impl fmt::Display for RpcConfigListReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.rpc_configs.is_empty() {
            return write!(
                f,
                "No RPC configs. Add one with `edw network rpc-config add <name> --url <url>`."
            );
        }
        let lines: Vec<String> = self
            .rpc_configs
            .iter()
            .map(|config| {
                let mark = if config.active { "*" } else { " " };
                format!("{mark} {config}")
            })
            .collect();
        write!(f, "{}", lines.join("\n"))
    }
}

impl Report for RpcConfigAddReport {
    const KIND: &'static str = "edw/network-rpc-config-add";
    const VERSION: u32 = 1;
}

impl fmt::Display for RpcConfigAddReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "added RPC config {}", self.rpc_config)
    }
}

impl Report for RpcConfigUseReport {
    const KIND: &'static str = "edw/network-rpc-config-use";
    const VERSION: u32 = 1;
}

impl fmt::Display for RpcConfigUseReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "active RPC config {}", self.name)
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List(args) => args.run(global).await,
            Self::Add(args) => args.run(global).await,
            Self::Use(args) => args.run(global).await,
        }
    }
}

impl RpcConfigListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let active = instance.active_rpc_config_name().await?;
        let rpc_configs = instance
            .rpc_configs()
            .await?
            .into_iter()
            .map(|config| {
                let active = active.as_deref() == Some(config.name.as_str());
                RpcConfigSummary::new(config, active)
            })
            .collect();
        RpcConfigListReport { rpc_configs }.emit(global.mode())
    }
}

impl RpcConfigAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let config = RpcConfig::http(self.name.clone(), &self.url, self.event_block_range)?;
        instance.add_rpc_config(config.clone()).await?;
        if self.r#use {
            instance.use_rpc_config(&self.name).await?;
        }
        let active =
            instance.active_rpc_config_name().await?.as_deref() == Some(self.name.as_str());
        RpcConfigAddReport {
            rpc_config: RpcConfigSummary::new(config, active),
        }
        .emit(global.mode())
    }
}

impl RpcConfigUseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        global.open().await?.use_rpc_config(&self.name).await?;
        RpcConfigUseReport {
            name: self.name.clone(),
        }
        .emit(global.mode())
    }
}
