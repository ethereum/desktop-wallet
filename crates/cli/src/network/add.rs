use std::{fmt, num::NonZeroU64};

use clap::{Args, ValueEnum};
use edw_core::network::{
    DEFAULT_EVENT_BLOCK_RANGE, DEFAULT_LOCAL_NODE_PORT, LocalNodeConfig, NetworkConfigKind,
    SimpleProviderConfig, db::NetworkDb,
};
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum NetworkConfigType {
    #[value(name = "simple-provider")]
    SimpleProvider,
    #[value(name = "local-node", alias = "local")]
    LocalNode,
}

#[derive(Args, Debug)]
pub struct NetworkAddArgs {
    name: String,
    /// `simple-provider` or `local-node` (alias: `local`)
    #[arg(value_enum, value_name = "TYPE")]
    kind: NetworkConfigType,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    event_block_range: Option<NonZeroU64>,
}

#[derive(Args, Debug)]
pub struct NetworkUseArgs {
    name: String,
}

#[derive(Serialize)]
struct NetworkAddReport {
    name: String,
    r#type: String,
    chain_id: u64,
    active: bool,
}

#[derive(Serialize)]
struct NetworkUseReport {
    name: String,
}

impl Report for NetworkAddReport {
    const KIND: &'static str = "edw/network-add";
    const VERSION: u32 = 1;
}

impl fmt::Display for NetworkAddReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "added {} {} (chain {})",
            self.name, self.r#type, self.chain_id
        )
    }
}

impl Report for NetworkUseReport {
    const KIND: &'static str = "edw/network-use";
    const VERSION: u32 = 1;
}

impl fmt::Display for NetworkUseReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "active {}", self.name)
    }
}

impl NetworkAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        if self.name.is_empty() {
            anyhow::bail!("networkConfig name cannot be empty");
        }

        let context = global.gather().await?;
        let mut configs = context.network_configs().await?;
        if configs.iter().any(|config| config.name == self.name) {
            anyhow::bail!("networkConfig `{}` already exists", self.name);
        }

        let event_block_range = self.event_block_range.unwrap_or(DEFAULT_EVENT_BLOCK_RANGE);
        let kind = match self.kind {
            NetworkConfigType::SimpleProvider => {
                if self.port.is_some() {
                    anyhow::bail!("--port is only valid for local-node");
                }
                let url = match &self.url {
                    Some(url) => http_url(url)?.to_string(),
                    None => context.network.default_rpc_url().to_string(),
                };
                NetworkConfigKind::SimpleProvider(SimpleProviderConfig {
                    url,
                    event_block_range,
                })
            }
            NetworkConfigType::LocalNode => {
                if self.url.is_some() {
                    anyhow::bail!("--url is only valid for simple-provider");
                }
                NetworkConfigKind::LocalNode(LocalNodeConfig {
                    port: self.port.unwrap_or(DEFAULT_LOCAL_NODE_PORT),
                    event_block_range,
                })
            }
        };

        let mut config = context.network.default_config();
        config.name = self.name.clone();
        config.config = kind;
        let report = NetworkAddReport {
            name: config.name.clone(),
            r#type: config.config.type_name().to_string(),
            chain_id: config.network_id.0,
            active: context.preferences_db().get_active().await?.is_none(),
        };
        configs.push(config);
        context.put_network_configs(&configs).await?;
        if report.active {
            context.preferences_db().put_active(&self.name).await?;
        }
        report.emit(global.mode())
    }
}

impl NetworkUseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        if self.name.is_empty() {
            anyhow::bail!("networkConfig name cannot be empty");
        }

        let context = global.gather().await?;
        let configs = context.network_configs().await?;
        if !configs.iter().any(|config| config.name == self.name) {
            anyhow::bail!("no networkConfig `{}`", self.name);
        }
        context.preferences_db().put_active(&self.name).await?;
        NetworkUseReport {
            name: self.name.clone(),
        }
        .emit(global.mode())
    }
}

fn http_url(url: &str) -> Result<&str, anyhow::Error> {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url)
    } else {
        anyhow::bail!("RPC URL must be an http or https URL")
    }
}
