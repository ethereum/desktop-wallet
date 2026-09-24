use clap::Subcommand;

use crate::{GlobalArgs, unlock};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Prints the unlocked network instance store path.
    Path,
    /// Applies pending database migrations.
    Migrate,
    /// Deletes profile database data.
    Purge,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Command::Path => {
                let context = global.gather().await?;
                println!(
                    "{}",
                    unlock::network_dir(&global.data_dir, context.network).display()
                );
                Ok(())
            }
            Command::Migrate => anyhow::bail!("database migrations are not implemented"),
            Command::Purge => anyhow::bail!("database purge is not implemented"),
        }
    }
}
