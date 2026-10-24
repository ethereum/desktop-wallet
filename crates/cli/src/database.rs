use std::fmt;

use clap::Subcommand;
use serde::Serialize;

use crate::{GlobalArgs, report::Report};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Prints the unlocked network instance store path.
    Path,
    /// Applies pending database migrations.
    #[command(hide = true)]
    Migrate,
    /// Deletes profile database data.
    #[command(hide = true)]
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
                let instance = global.open().await?;
                DatabasePathReport {
                    path: global
                        .data_dir()
                        .instance_dir(instance.network().id)
                        .display()
                        .to_string(),
                }
                .emit(global.mode())
            }
            Command::Migrate => anyhow::bail!("database migrations are not implemented"),
            Command::Purge => anyhow::bail!("database purge is not implemented"),
        }
    }
}
