use std::path::PathBuf;

use anyhow::Context as _;
use clap::Args;
use edw_core::instance::{DataDir, Instance};

use crate::session::Session;

#[derive(Args)]
pub struct GlobalArgs {
    #[arg(long, global = true, env = "DATA_DIR", default_value = "./.edw/")]
    data_dir: PathBuf,
    /// Overrides the unlocked network's endpoint for this invocation.
    #[arg(long, global = true, env = "RPC_URL")]
    pub rpc_url: Option<String>,
}

impl GlobalArgs {
    pub fn data_dir(&self) -> DataDir {
        DataDir::new(&self.data_dir)
    }

    /// Opens the instance the session holds unlocked.
    pub async fn open(&self) -> anyhow::Result<Instance> {
        let session = Session::load().context("wallet is locked; run `edw unlock`")?;
        let data_dir = self.data_dir();
        if session.data_dir != data_dir.path() {
            anyhow::bail!(
                "wallet is unlocked for {} at {}, not {}; run `edw unlock [--network <name or id>]`",
                session.network,
                session.data_dir.display(),
                data_dir.path().display()
            );
        }
        Ok(Instance::open(&data_dir, session.network, session.password.as_bytes()).await?)
    }
}
