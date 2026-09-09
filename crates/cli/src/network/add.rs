use clap::Args;
use edw_core::network::{Network, db::NetworkDb};

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkAddArgs {
    /// A preset slug or network ID.
    id_or_preset: String,
    /// A display name, required when adding a bare network ID.
    #[arg(long)]
    name: Option<String>,
    /// An RPC endpoint to configure for this network.
    #[arg(long, value_name = "RPC_URL")]
    rpc_url: Option<String>,
}

impl NetworkAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let context = global.gather().await?;
        let mut networks = context.networks.get_networks().await?;

        let incoming = Network::from_preset(&self.id_or_preset)?;

        if let Some(index) = networks
            .iter()
            .position(|n| n.network_id == incoming.network_id)
        {
            let existing = &mut networks[index];
            println!(
                "updated {} (chain {})",
                existing.name, existing.network_id.0
            );
        } else {
            println!("added {} (chain {})", incoming.name, incoming.network_id.0);
            networks.push(incoming);
        }

        context.networks.put_networks(&networks).await?;

        Ok(())
    }
}
