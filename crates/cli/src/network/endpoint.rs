use std::{fmt, num::NonZeroU64};

use clap::{Args, Subcommand};
use edw_core::network::{DEFAULT_EVENT_BLOCK_RANGE, NetworkEndpointConfig, NetworkEndpointKind};
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List this network's endpoints. The active one is marked `*`.
    List(EndpointListArgs),
    /// Add an endpoint. The first one added becomes active.
    Add(EndpointAddArgs),
    /// Make an endpoint the active one.
    Use(EndpointUseArgs),
}

#[derive(Args, Debug)]
pub struct EndpointListArgs {}

#[derive(Args, Debug)]
pub struct EndpointAddArgs {
    name: String,
    /// http or https JSON-RPC URL.
    #[arg(long)]
    url: String,
    /// The widest block span one log request may ask this endpoint for.
    #[arg(long, default_value_t = DEFAULT_EVENT_BLOCK_RANGE)]
    event_block_range: NonZeroU64,
}

#[derive(Args, Debug)]
pub struct EndpointUseArgs {
    name: String,
}

#[derive(Serialize)]
struct EndpointSummary {
    name: String,
    kind: &'static str,
    url: String,
    active: bool,
}

#[derive(Serialize)]
struct EndpointListReport {
    endpoints: Vec<EndpointSummary>,
}

#[derive(Serialize)]
struct EndpointAddReport {
    endpoint: EndpointSummary,
}

#[derive(Serialize)]
struct EndpointUseReport {
    name: String,
}

impl EndpointSummary {
    fn new(config: NetworkEndpointConfig, active: bool) -> Self {
        let (kind, url) = match config.kind {
            NetworkEndpointKind::Http { url } => ("http", url),
        };
        Self {
            name: config.name,
            kind,
            url,
            active,
        }
    }
}

impl fmt::Display for EndpointSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.name, self.kind, self.url)
    }
}

impl Report for EndpointListReport {
    const KIND: &'static str = "edw/network-endpoint-list";
    const VERSION: u32 = 1;
}

impl fmt::Display for EndpointListReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.endpoints.is_empty() {
            return write!(
                f,
                "No endpoints. Add one with `edw network endpoint add <name> --url <url>`."
            );
        }
        let lines: Vec<String> = self
            .endpoints
            .iter()
            .map(|endpoint| {
                let mark = if endpoint.active { "*" } else { " " };
                format!("{mark} {endpoint}")
            })
            .collect();
        write!(f, "{}", lines.join("\n"))
    }
}

impl Report for EndpointAddReport {
    const KIND: &'static str = "edw/network-endpoint-add";
    const VERSION: u32 = 1;
}

impl fmt::Display for EndpointAddReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "added endpoint {}", self.endpoint)
    }
}

impl Report for EndpointUseReport {
    const KIND: &'static str = "edw/network-endpoint-use";
    const VERSION: u32 = 1;
}

impl fmt::Display for EndpointUseReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "active endpoint {}", self.name)
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

impl EndpointListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let active = instance.active_endpoint_name().await?;
        let endpoints = instance
            .endpoint_configs()
            .await?
            .into_iter()
            .map(|config| {
                let active = active.as_deref() == Some(config.name.as_str());
                EndpointSummary::new(config, active)
            })
            .collect();
        EndpointListReport { endpoints }.emit(global.mode())
    }
}

impl EndpointAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let config =
            NetworkEndpointConfig::http(self.name.clone(), &self.url, self.event_block_range)?;
        instance.add_endpoint_config(config.clone()).await?;
        let active = instance.active_endpoint_name().await?.as_deref() == Some(self.name.as_str());
        EndpointAddReport {
            endpoint: EndpointSummary::new(config, active),
        }
        .emit(global.mode())
    }
}

impl EndpointUseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        global.open().await?.use_endpoint_config(&self.name).await?;
        EndpointUseReport {
            name: self.name.clone(),
        }
        .emit(global.mode())
    }
}
