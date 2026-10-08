use std::num::NonZeroU64;

use clap::{Args, Subcommand};
use edw_core::network::{DEFAULT_EVENT_BLOCK_RANGE, NetworkEndpointConfig};

use crate::GlobalArgs;

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
        let configs = instance.endpoint_configs().await?;
        if configs.is_empty() {
            println!("No endpoints. Add one with `edw network endpoint add <name> --url <url>`.");
            return Ok(());
        }

        let active = instance.active_endpoint_name().await?;
        for config in &configs {
            let mark = if active.as_deref() == Some(config.name.as_str()) {
                "*"
            } else {
                " "
            };
            println!("{mark} {} {}", config.name, config.kind);
        }
        Ok(())
    }
}

impl EndpointAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let config =
            NetworkEndpointConfig::http(self.name.clone(), &self.url, self.event_block_range)?;
        let kind = config.kind.to_string();
        instance.add_endpoint_config(config).await?;
        println!("added endpoint {} {kind}", self.name);
        Ok(())
    }
}

impl EndpointUseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        global.open().await?.use_endpoint_config(&self.name).await?;
        println!("active endpoint {}", self.name);
        Ok(())
    }
}
