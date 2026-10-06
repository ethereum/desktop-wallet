use clap::Args;
use edw_core::network::db::NetworkDb;

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkListArgs {}

impl NetworkListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let context = global.gather().await?;
        let configs = context.network_configs().await?;
        let active = context.preferences_db().get_active().await?;

        if configs.is_empty() {
            println!("No networkConfigs.");
            println!("Add one with `edw network add <name> <type>`.");
            return Ok(());
        }

        for config in &configs {
            let mark = if active.as_deref() == Some(config.name.as_str()) {
                "*"
            } else {
                " "
            };
            println!(
                "{mark} {} {} (network id {})",
                config.name,
                config.config.type_name(),
                config.network_id.0
            );
        }
        Ok(())
    }
}
