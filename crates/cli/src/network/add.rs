use clap::Args;
use edw_core::network::{db::NetworkDb, endpoint::NetworkEndpointConfig};

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkSetRpcArgs {
    url: String,
}

impl NetworkSetRpcArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let url = self.url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            anyhow::bail!("RPC URL must be an http or https URL");
        }

        let context = global.gather().await?;
        let db = context.preferences_db();
        let mut network = context.preferences().await?;
        network.endpoints = vec![NetworkEndpointConfig::HttpProvider {
            url: url.to_string(),
        }];
        db.put_network(&network).await?;
        println!("updated {} (chain {})", network.name, network.network_id.0);
        Ok(())
    }
}
