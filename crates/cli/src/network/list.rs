use edw_core::network::db::NetworkDb;

use crate::GlobalArgs;

pub async fn run(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let configs = context.network_configs().await?;
    let active = context.preferences_db().get_active().await?;

    if configs.is_empty() {
        println!("No networkConfigs.");
        println!("Add one with `edw network add <name>`.");
        return Ok(());
    }

    for config in &configs {
        let mark = if active.as_deref() == Some(config.name.as_str()) {
            "*"
        } else {
            " "
        };
        println!("{mark} {} (chain {})", config.name, config.network_id.0);
    }
    Ok(())
}
