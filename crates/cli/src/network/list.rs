use std::fmt;

use edw_core::network::db::NetworkDb;
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Serialize)]
struct NetworkConfigsReport {
    configs: Vec<NetworkConfigRow>,
}

#[derive(Serialize)]
struct NetworkConfigRow {
    name: String,
    r#type: String,
    chain_id: u64,
    active: bool,
}

impl Report for NetworkConfigsReport {
    const KIND: &'static str = "edw/network-view";
    const VERSION: u32 = 1;
}

impl fmt::Display for NetworkConfigsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.configs.is_empty() {
            writeln!(f, "No networkConfigs.")?;
            return write!(f, "Add one with `edw network add <name> <type>`.");
        }

        let rows = self.configs.iter().map(|config| {
            let mark = if config.active { "*" } else { " " };
            format!(
                "{mark} {} {} (chain {})",
                config.name, config.r#type, config.chain_id
            )
        });
        write!(f, "{}", rows.collect::<Vec<_>>().join("\n"))
    }
}

pub async fn run(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let context = global.gather().await?;
    let configs = context.network_configs().await?;
    let active = context.preferences_db().get_active().await?;

    let report = NetworkConfigsReport {
        configs: configs
            .iter()
            .map(|config| NetworkConfigRow {
                name: config.name.clone(),
                r#type: config.config.type_name().to_string(),
                chain_id: config.network_id.0,
                active: active.as_deref() == Some(config.name.as_str()),
            })
            .collect(),
    };

    report.emit(global.mode())
}
