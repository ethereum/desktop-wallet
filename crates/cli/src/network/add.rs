use clap::{Args, ValueEnum};
use edw_core::network::{
    DEFAULT_EVENT_BLOCK_RANGE, DEFAULT_LOCAL_NODE_PORT, LocalNodeConfig, NetworkConfigKind,
    SimpleProviderConfig, db::NetworkDb,
};

use crate::GlobalArgs;

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
    event_block_range: Option<u64>,
}

#[derive(Args, Debug)]
pub struct NetworkUseArgs {
    name: String,
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
        if event_block_range == 0 {
            anyhow::bail!("--event-block-range must be at least 1");
        }
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
        println!(
            "added {} {} (chain {})",
            config.name,
            config.config.type_name(),
            config.network_id.0
        );
        configs.push(config);
        context.put_network_configs(&configs).await?;
        if context.preferences_db().get_active().await?.is_none() {
            context.preferences_db().put_active(&self.name).await?;
        }
        Ok(())
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
        println!("active {}", self.name);
        Ok(())
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
