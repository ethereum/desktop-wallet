use std::num::NonZeroU64;

use clap::{Args, ValueEnum};
use edw_core::instance::NetworkConfigSpec;

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
    event_block_range: Option<NonZeroU64>,
}

#[derive(Args, Debug)]
pub struct NetworkUseArgs {
    name: String,
}

impl NetworkAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let spec = match self.kind {
            NetworkConfigType::SimpleProvider => {
                if self.port.is_some() {
                    anyhow::bail!("--port is only valid for local-node");
                }
                NetworkConfigSpec::SimpleProvider {
                    url: self.url.clone(),
                }
            }
            NetworkConfigType::LocalNode => {
                if self.url.is_some() {
                    anyhow::bail!("--url is only valid for simple-provider");
                }
                NetworkConfigSpec::LocalNode { port: self.port }
            }
        };

        let config = global
            .open()
            .await?
            .add_network_config(self.name.clone(), spec, self.event_block_range)
            .await?;
        println!(
            "added {} {} (network id {})",
            config.name, config.config, config.network_id.0
        );
        Ok(())
    }
}

impl NetworkUseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        global.open().await?.use_network_config(&self.name).await?;
        println!("active {}", self.name);
        Ok(())
    }
}
