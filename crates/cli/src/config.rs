use std::fmt;

use clap::Subcommand;
use edw_core::{
    instance::{DataDir, Instance},
    network::RpcConfigKind,
};
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    session::{Session, SessionFile},
};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Prints the resolved data directory.
    Path,
    /// Prints the data directory, its network instances, the session, and the active RPC config.
    View,
}

#[derive(Serialize)]
struct ConfigPathReport {
    data_dir: String,
}

#[derive(Serialize)]
struct ConfigViewReport {
    data_dir: String,
    instances: Vec<String>,
    locked: bool,
    session: Option<SessionView>,
    rpc_config: Option<RpcConfigView>,
}

#[derive(Serialize)]
struct SessionView {
    network: String,
    data_dir: String,
}

#[derive(Serialize)]
struct RpcConfigView {
    name: String,
    url: String,
}

impl Report for ConfigPathReport {
    const KIND: &'static str = "edw/config-path";
    const VERSION: u32 = 1;
}

impl fmt::Display for ConfigPathReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.data_dir)
    }
}

impl Report for ConfigViewReport {
    const KIND: &'static str = "edw/config-view";
    const VERSION: u32 = 3;
}

impl fmt::Display for ConfigViewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "data_dir={}", self.data_dir)?;
        if self.instances.is_empty() {
            writeln!(f, "instances=(none)")?;
        } else {
            writeln!(f, "instances={}", self.instances.join(", "))?;
        }
        match (&self.session, self.locked) {
            (Some(session), false) => writeln!(f, "session={}", session.network)?,
            (Some(session), true) => writeln!(
                f,
                "session=(locked here; held for {} at {})",
                session.network, session.data_dir
            )?,
            (None, _) => writeln!(f, "session=(locked)")?,
        }
        match &self.rpc_config {
            Some(config) => write!(f, "rpc_config={} {}", config.name, config.url),
            None if self.locked => write!(f, "rpc_config=(unknown while locked)"),
            None => write!(f, "rpc_config=(none)"),
        }
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Command::Path => ConfigPathReport {
                data_dir: global.data_dir().path().display().to_string(),
            }
            .emit(global.mode()),
            Command::View => view(&global.data_dir()).await?.emit(global.mode()),
        }
    }
}

/// Reads the configuration without prompting, so it works while the wallet is locked.
async fn view(data_dir: &DataDir) -> Result<ConfigViewReport, anyhow::Error> {
    let session = SessionFile::runtime().load_if_present().await;
    let unlocked = session
        .as_ref()
        .filter(|session| session.data_dir == data_dir.path());
    let rpc_config = match unlocked {
        Some(session) => active_rpc_config(data_dir, session).await?,
        None => None,
    };
    Ok(ConfigViewReport {
        data_dir: data_dir.path().display().to_string(),
        instances: data_dir
            .instances()
            .iter()
            .map(ToString::to_string)
            .collect(),
        locked: unlocked.is_none(),
        session: session.map(|session| SessionView {
            network: session.network.to_string(),
            data_dir: session.data_dir.display().to_string(),
        }),
        rpc_config,
    })
}

async fn active_rpc_config(
    data_dir: &DataDir,
    session: &Session,
) -> Result<Option<RpcConfigView>, anyhow::Error> {
    let instance = Instance::open(data_dir, session.network, session.password.as_bytes()).await?;
    let Some(name) = instance.active_rpc_config_name().await? else {
        return Ok(None);
    };
    Ok(instance
        .rpc_configs()
        .await?
        .into_iter()
        .find(|config| config.name == name)
        .map(|config| match config.kind {
            RpcConfigKind::Http { url } => RpcConfigView {
                name: config.name,
                url,
            },
        }))
}
