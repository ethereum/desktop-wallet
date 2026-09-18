use clap::Args;
use edw_core::network::{db::NetworkDb, endpoint::NetworkEndpointConfig};

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkAddArgs {
    name: String,
    #[arg(long)]
    rpc_url: Option<String>,
}

#[derive(Args, Debug)]
pub struct NetworkSetRpcArgs {
    url: String,
    #[arg(long)]
    name: Option<String>,
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

        let mut config = context.network.default_config();
        config.name = self.name.clone();
        config.endpoints = match &self.rpc_url {
            Some(url) => vec![NetworkEndpointConfig::HttpProvider {
                url: http_url(url)?.to_string(),
            }],
            None => vec![],
        };
        println!("added {} (chain {})", config.name, config.network_id.0);
        configs.push(config);
        context.put_network_configs(&configs).await?;
        if context.preferences_db().get_active().await?.is_none() {
            context.preferences_db().put_active(&self.name).await?;
        }
        Ok(())
    }
}

impl NetworkSetRpcArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let url = http_url(&self.url)?;
        let context = global.gather().await?;
        let (index, mut configs) = context.resolve_config(self.name.as_deref()).await?;
        configs[index].endpoints = vec![NetworkEndpointConfig::HttpProvider {
            url: url.to_string(),
        }];
        let name = configs[index].name.clone();
        let chain = configs[index].network_id.0;
        context.put_network_configs(&configs).await?;
        println!("updated {name} (chain {chain})");
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
