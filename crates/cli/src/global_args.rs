use std::path::PathBuf;

use clap::Args;
use clap_verbosity_flag::{Verbosity, WarnLevel};
use edw_core::instance::{DataDir, Instance};

use crate::{failure, input::Input, report::Mode, session::SessionFile, unlock};

#[derive(Args)]
pub struct GlobalArgs {
    #[arg(long, global = true, env = "EDW_DATA_DIR", default_value_os_t = default_data_dir())]
    data_dir: PathBuf,
    /// Never prompt, and print one JSON document on stdout. Anything a command needs must come
    /// from flags, the environment, the session, or piped stdin.
    #[arg(long, global = true, visible_alias = "porcelain")]
    pub non_interactive: bool,
    #[command(flatten)]
    pub verbosity: Verbosity<WarnLevel>,
}

impl GlobalArgs {
    pub fn data_dir(&self) -> DataDir {
        DataDir::new(&self.data_dir)
    }

    pub const fn mode(&self) -> Mode {
        if self.non_interactive {
            Mode::Json
        } else {
            Mode::Human
        }
    }

    pub fn input(&self) -> Input {
        Input::detect(self.non_interactive)
    }

    /// Opens the instance the session holds unlocked. Without one, prompts for the password
    /// at an interactive terminal and fails anywhere else, so a script never waits on a prompt.
    pub async fn open(&self) -> anyhow::Result<Instance> {
        let data_dir = self.data_dir();
        let session = SessionFile::runtime().load_if_present().await;
        if let Some(session) = &session
            && session.data_dir == data_dir.path()
        {
            return Ok(
                Instance::open(&data_dir, session.network, session.password.as_bytes()).await?,
            );
        }

        if self.input() == Input::Terminal {
            return unlock::prompt_unlock(&data_dir, Input::Terminal).await;
        }
        match session {
            Some(session) => Err(failure::locked(format!(
                "wallet is unlocked for {} at {}, not {}; run `edw unlock [--network <name or id>]`",
                session.network,
                session.data_dir.display(),
                data_dir.path().display()
            ))),
            None => Err(failure::locked("wallet is locked; run `edw unlock`")),
        }
    }
}

fn default_data_dir() -> PathBuf {
    std::env::home_dir().map_or_else(|| PathBuf::from("./.edw/"), |home| home.join(".edw"))
}
