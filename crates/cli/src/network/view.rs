use std::fmt;

use clap::Args;
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Args, Debug)]
pub struct NetworkViewArgs {}

#[derive(Serialize)]
struct NetworkViewReport {
    network: String,
    network_id: u64,
    native_asset: String,
    active_endpoint: Option<String>,
}

impl Report for NetworkViewReport {
    const KIND: &'static str = "edw/network-view";
    const VERSION: u32 = 1;
}

impl fmt::Display for NetworkViewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "network {} (network id {})",
            self.network, self.network_id
        )?;
        writeln!(f, "native asset {}", self.native_asset)?;
        match &self.active_endpoint {
            Some(name) => write!(f, "active endpoint {name}"),
            None => write!(
                f,
                "no active endpoint; add one with `edw network endpoint add`"
            ),
        }
    }
}

impl NetworkViewArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let network = instance.network();
        NetworkViewReport {
            network: network.id.to_string(),
            network_id: network.id.0,
            native_asset: network.native_asset.clone(),
            active_endpoint: instance.active_endpoint_name().await?,
        }
        .emit(global.mode())
    }
}
