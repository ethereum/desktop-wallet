use clap::Args;

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkViewArgs {}

impl NetworkViewArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let network = instance.network();
        println!("network {} (network id {})", network.id, network.id.0);
        println!("native asset {}", network.native_asset);
        match instance.active_endpoint_name().await? {
            Some(name) => println!("active endpoint {name}"),
            None => println!("no active endpoint; add one with `edw network endpoint add`"),
        }
        Ok(())
    }
}
