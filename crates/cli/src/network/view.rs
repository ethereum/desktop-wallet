use std::fmt;

use clap::Args;
use edw_core::network::RpcConfigKind;
use serde::Serialize;

use super::rpc_config::interval_label;
use crate::{GlobalArgs, report::Report};

#[derive(Args, Debug)]
pub struct NetworkViewArgs {}

#[derive(Serialize)]
struct EndpointLine {
    service: String,
    name: Option<String>,
    url: Option<String>,
    active: bool,
    block_range: Option<u64>,
    interval_ms: Option<u64>,
}

#[derive(Serialize)]
struct NetworkViewReport {
    network: String,
    network_id: u64,
    native_asset: String,
    endpoints: Vec<EndpointLine>,
}

impl Report for NetworkViewReport {
    const KIND: &'static str = "edw/network-view";
    const VERSION: u32 = 3;
}

impl fmt::Display for NetworkViewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "network {} (network id {})",
            self.network, self.network_id
        )?;
        writeln!(f, "native asset {}", self.native_asset)?;
        if !self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.service == "rpc")
        {
            writeln!(
                f,
                "no active RPC config; add one with `edw network rpc-config add <name> --url <url>`"
            )?;
        }
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            let line = endpoint_line(endpoint);
            if index + 1 == self.endpoints.len() {
                write!(f, "{line}")?;
            } else {
                writeln!(f, "{line}")?;
            }
        }
        Ok(())
    }
}

impl NetworkViewArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let network = instance.network();
        let mut endpoints = Vec::new();
        if let Some(config) = instance.active_rpc().await? {
            let url = match config.kind {
                RpcConfigKind::Http { url } => url,
            };
            endpoints.push(EndpointLine {
                service: "rpc".to_string(),
                name: Some(config.name),
                url: Some(url),
                active: true,
                block_range: Some(config.event_block_range.get()),
                interval_ms: Some(config.call_interval_ms),
            });
        }
        let bundlers = instance.bundler_urls().await?;
        if bundlers.is_empty() {
            endpoints.push(EndpointLine {
                service: "bundler".to_string(),
                name: None,
                url: None,
                active: false,
                block_range: None,
                interval_ms: None,
            });
        } else {
            for url in bundlers {
                endpoints.push(EndpointLine {
                    service: "bundler".to_string(),
                    name: None,
                    url: Some(url.to_string()),
                    active: true,
                    block_range: None,
                    interval_ms: None,
                });
            }
        }
        let indexer = instance.indexer_config().await?;
        let indexer_active = indexer.enabled && indexer.url.is_some();
        endpoints.push(EndpointLine {
            service: "indexer".to_string(),
            name: None,
            url: indexer.url,
            active: indexer_active,
            block_range: None,
            interval_ms: None,
        });
        NetworkViewReport {
            network: network.id.to_string(),
            network_id: network.id.0,
            native_asset: network.native_asset.clone(),
            endpoints,
        }
        .emit(global.mode())
    }
}

fn endpoint_line(endpoint: &EndpointLine) -> String {
    let state = if endpoint.active {
        "active"
    } else {
        "inactive"
    };
    match endpoint.service.as_str() {
        "rpc" => format!(
            "rpc {} {} blockrange {} interval {} {state}",
            endpoint.name.as_deref().unwrap_or(""),
            endpoint.url.as_deref().unwrap_or(""),
            endpoint.block_range.unwrap_or(0),
            interval_label(endpoint.interval_ms.unwrap_or(0))
        ),
        "indexer" => match &endpoint.url {
            Some(url) => format!("indexer {url} {state}"),
            None => format!("indexer {state}"),
        },
        "bundler" => match &endpoint.url {
            Some(url) => format!("bundler {url} {state}"),
            None => format!("bundler {state}"),
        },
        other => format!("{other} {state}"),
    }
}
