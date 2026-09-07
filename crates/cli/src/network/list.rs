use clap::Args;
use edw_core::network::db::NetworkDb;

use crate::{GlobalArgs, utils::table::table};

#[derive(Args, Debug)]
pub struct NetworkListArgs {
    /// Prints endpoint URLs in full, including any credentials they carry.
    #[arg(long)]
    show_urls: bool,
}

impl NetworkListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let context = global.gather().await?;
        let networks = context.networks.get_networks().await?;

        if networks.is_empty() {
            println!("No networks configured.");
            println!("Add one with `edw network add <network> --rpc-url <url>`.");
            return Ok(());
        }

        let rows = networks
            .iter()
            .map(|network| {
                vec![
                    network.name.clone(),
                    network.network_id.0.to_string(),
                    network.native_token.clone(),
                ]
            })
            .collect::<Vec<_>>();

        table(&["", "NAME", "NETWORK ID", "TOKEN"], &rows);
        Ok(())
    }
}
