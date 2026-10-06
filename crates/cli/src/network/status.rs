use clap::Args;

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkStatusArgs {}

impl NetworkStatusArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let network = instance.network();
        println!("network {} (network id {})", network.id, network.id.0);
        println!("native asset {}", network.native_asset);

        let name = match (&global.rpc_url, instance.active_endpoint_name().await?) {
            (Some(url), _) => url.clone(),
            (None, Some(name)) => name,
            (None, None) => {
                println!("no endpoint; add one with `edw network endpoint add <name> --url <url>`");
                return Ok(());
            }
        };
        let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
        println!(
            "endpoint {name} at block {}",
            endpoint.block_height().await?
        );
        Ok(())
    }
}
