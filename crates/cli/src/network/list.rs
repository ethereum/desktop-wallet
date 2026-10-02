use std::io::{self, Write};

use edw_core::network::db::NetworkDb;
use serde::Serialize;

use crate::{
    GlobalArgs,
    output::{self, Report},
};

#[derive(Serialize)]
pub(crate) struct NetworkConfigsReport {
    active: Option<String>,
    configs: Vec<NetworkConfigRow>,
}

#[derive(Serialize)]
struct NetworkConfigRow {
    name: String,
    kind: String,
    chain_id: u64,
    active: bool,
}

impl Report for NetworkConfigsReport {
    fn render(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.configs.is_empty() {
            writeln!(out, "No networkConfigs.")?;
            return writeln!(out, "Add one with `edw network add <name> <type>`.");
        }

        for config in &self.configs {
            let mark = if config.active { "*" } else { " " };
            writeln!(
                out,
                "{mark} {} {} (chain {})",
                config.name, config.kind, config.chain_id
            )?;
        }
        Ok(())
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
                kind: config.config.type_name().to_string(),
                chain_id: config.network_id.0,
                active: active.as_deref() == Some(config.name.as_str()),
            })
            .collect(),
        active,
    };

    output::emit(global.mode(), &report)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn report(rows: Vec<NetworkConfigRow>, active: Option<&str>) -> NetworkConfigsReport {
        NetworkConfigsReport {
            active: active.map(str::to_string),
            configs: rows,
        }
    }

    fn row(name: &str, active: bool) -> NetworkConfigRow {
        NetworkConfigRow {
            name: name.into(),
            kind: "local-node".into(),
            chain_id: 31_337,
            active,
        }
    }

    fn rendered(report: &NetworkConfigsReport) -> String {
        let mut buffer = Vec::new();
        report.render(&mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn only_the_active_config_carries_the_mark() {
        let rendered = rendered(&report(
            vec![row("default", false), row("devnet", true)],
            Some("devnet"),
        ));

        let marked: Vec<&str> = rendered
            .lines()
            .filter(|line| line.starts_with('*'))
            .collect();

        assert_eq!(marked.len(), 1, "{rendered}");
        assert!(marked[0].contains("devnet"), "{rendered}");
    }

    #[test]
    fn no_configs_tells_a_person_how_to_add_one() {
        let rendered = rendered(&report(vec![], None));

        assert!(rendered.contains("No networkConfigs."), "{rendered}");
        assert!(rendered.contains("edw network add"), "{rendered}");
    }
}
