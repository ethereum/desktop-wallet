use std::fmt;

use clap::Subcommand;
use edw_core::instance::{DataDir, Instance};
use serde::Serialize;

use crate::{GlobalArgs, report::Report, session::SessionFile, unlock};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Prints the resolved data directory.
    Path,
    /// Prints the data directory, its network instances, the session, and the unlocked instance's
    /// directory.
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
    instance_dir: Option<String>,
}

#[derive(Serialize)]
struct SessionView {
    network: String,
    data_dir: String,
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
    const VERSION: u32 = 4;
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
            (None, false) => writeln!(f, "session=(none; opened with EDW_DECRYPTION_PASSWORD)")?,
            (None, true) => writeln!(f, "session=(locked)")?,
        }
        match &self.instance_dir {
            Some(dir) => write!(f, "instance_dir={dir}"),
            None => write!(f, "instance_dir=(unknown while locked)"),
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
            Command::View => {
                let password = unlock::env_password()?;
                view(
                    &global.data_dir(),
                    &SessionFile::runtime(),
                    password.as_deref().map(String::as_str),
                )
                .await?
                .emit(global.mode())
            }
        }
    }
}

/// Reads the configuration without prompting, so it works while the wallet is locked.
async fn view(
    data_dir: &DataDir,
    sessions: &SessionFile,
    password: Option<&str>,
) -> Result<ConfigViewReport, anyhow::Error> {
    let mut session = sessions.load_if_present().await;
    let mut instance = None;
    if let Some(held) = &session
        && held.data_dir == data_dir.path()
    {
        instance = sessions.open(held).await?;
        if instance.is_none() {
            session = None;
        }
    }
    if instance.is_none() {
        instance = open_only_instance(data_dir, password).await?;
    }
    let instance_dir = instance.as_ref().map(|instance| {
        data_dir
            .instance_dir(instance.network().id)
            .display()
            .to_string()
    });
    Ok(ConfigViewReport {
        data_dir: data_dir.path().display().to_string(),
        instances: data_dir
            .instances()
            .iter()
            .map(ToString::to_string)
            .collect(),
        locked: instance.is_none(),
        session: session.map(|session| SessionView {
            network: session.network.to_string(),
            data_dir: session.data_dir.display().to_string(),
        }),
        instance_dir,
    })
}

async fn open_only_instance(
    data_dir: &DataDir,
    password: Option<&str>,
) -> Result<Option<Instance>, anyhow::Error> {
    let instances = data_dir.instances();
    let (Some(password), [network]) = (password, instances.as_slice()) else {
        return Ok(None);
    };
    Ok(Some(
        Instance::open(data_dir, *network, password.as_bytes()).await?,
    ))
}

#[cfg(test)]
mod tests {
    use edw_core::network::NetworkId;

    use super::*;

    #[tokio::test]
    async fn without_a_session_the_password_finds_the_only_instance() {
        let dir = std::env::temp_dir().join(format!("edw-config-{}", std::process::id()));
        let data_dir = DataDir::new(&dir);
        Instance::open_or_create(&data_dir, NetworkId(31_337), b"secret")
            .await
            .unwrap();

        let report = view(&data_dir, &SessionFile::unavailable(), Some("secret"))
            .await
            .unwrap();

        assert_eq!(
            report.instance_dir,
            Some(
                data_dir
                    .instance_dir(NetworkId(31_337))
                    .display()
                    .to_string()
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
