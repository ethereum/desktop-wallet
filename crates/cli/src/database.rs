use std::fmt::Debug;

use clap::{Args, Subcommand};

use crate::{GlobalArgs, print_help};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Prints the profile database path pattern.
    Path,
    /// Applies pending database migrations.
    Migrate,
    /// Deletes profile database data.
    Purge,
}

#[derive(Debug, Args)]
pub struct DatabaseArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

impl DatabaseArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self.command {
            None => print_help("database"),
            Some(Command::Path) => {
                if let Ok(_ctx) = global.gather().await {
                    println!("{}/*/db", global.data_dir.display());
                    Ok(())
                } else {
                    anyhow::bail!("not implemented")
                }
            }
            Some(Command::Migrate) => anyhow::bail!("database migrations are not implemented"),
            Some(Command::Purge) => anyhow::bail!("database purge is not implemented"),
        }
    }
}
