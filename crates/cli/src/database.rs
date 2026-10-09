use std::fmt;

use clap::Subcommand;
use serde::Serialize;

use crate::{GlobalArgs, report::Report, unlock};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Prints the unlocked network instance store path.
    Path,
    /// Applies pending database migrations.
    Migrate,
    /// Deletes profile database data.
    Purge,
}

#[derive(Serialize)]
struct DatabasePathReport {
    path: String,
}

impl Report for DatabasePathReport {
    const KIND: &'static str = "edw/database-path";
    const VERSION: u32 = 1;
}

impl fmt::Display for DatabasePathReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)
    }
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Command::Path => {
                let context = global.gather().await?;
                let path = unlock::network_dir(&global.data_dir, context.network);
                DatabasePathReport {
                    path: path.display().to_string(),
                }
                .emit(global.mode())
            }
            Command::Migrate => anyhow::bail!("database migrations are not implemented"),
            Command::Purge => anyhow::bail!("database purge is not implemented"),
        }
    }
}
