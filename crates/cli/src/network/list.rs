use clap::Args;

use crate::GlobalArgs;

#[derive(Args, Debug)]
pub struct NetworkListArgs {}

impl NetworkListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let configs = instance.network_configs().await?;
        let active = instance.active_network_config_name().await?;

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
                config.name, config.config, config.network_id.0
            );
        }
        Ok(())
    }
}
