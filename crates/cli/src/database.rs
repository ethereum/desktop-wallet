use std::fmt::Debug;

use clap::Subcommand;

use crate::GlobalArgs;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Prints the profile database path pattern.
    Path,
    /// Applies pending database migrations.
    Migrate,
    /// Deletes profile database data.
    Purge,
}

impl Command {
    pub fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Command::Path => {
                println!("{}/*/db", global.data_dir.display());
                Ok(())
            }
            Command::Migrate => anyhow::bail!("database migrations are not implemented"),
            Command::Purge => anyhow::bail!("database purge is not implemented"),
        }
    }
}
