use std::path::PathBuf;

use clap::Args;
use clap_verbosity_flag::{Verbosity, WarnLevel};
use edw_core::instance::{DataDir, Instance};

use crate::{
    failure,
    input::Input,
    report::Mode,
    session::{self, SessionFile},
    unlock,
};

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

    /// Opens the session's instance, else unlocks it from `EDW_DECRYPTION_PASSWORD` or a terminal
    /// prompt, keeping a session for the prompt. Piped stdin is left to the command's fields.
    pub async fn open(&self) -> anyhow::Result<Instance> {
        let data_dir = self.data_dir();
        let sessions = SessionFile::runtime();
        let mut session = sessions.load_if_present().await;
        if let Some(held) = &session
            && held.data_dir == data_dir.path()
        {
            if let Some(instance) = sessions.open(held).await? {
                return Ok(instance);
            }
            session = None;
        }

        let input = self.input();
        if let Some(password) = unlock::env_password()? {
            let network = unlock::choose_network(&data_dir, input)?;
            return Ok(Instance::open(&data_dir, network, password.as_bytes()).await?);
        }
        if input == Input::Terminal {
            return unlock::prompt_unlock(&data_dir, Input::Terminal).await;
        }
        match session {
            Some(session) => Err(failure::locked(format!(
                "wallet is unlocked for {} at {}, not {}; run `edw unlock [--network <name or id>]` or set EDW_DECRYPTION_PASSWORD",
                session.network,
                session.data_dir.display(),
                data_dir.path().display()
            ))),
            None if session::can_hold() => Err(failure::locked(
                "wallet is locked; run `edw unlock` or set EDW_DECRYPTION_PASSWORD",
            )),
            None => Err(failure::locked(
                "wallet is locked, and no session can be held here; set EDW_DECRYPTION_PASSWORD",
            )),
        }
    }
}

fn default_data_dir() -> PathBuf {
    std::env::home_dir().map_or_else(|| PathBuf::from("./.edw/"), |home| home.join(".edw"))
}
