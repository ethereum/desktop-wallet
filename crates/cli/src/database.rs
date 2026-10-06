use clap::Subcommand;

use crate::GlobalArgs;

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
                let instance = global.open().await?;
                println!(
                    "{}",
                    global
                        .data_dir()
                        .instance_dir(instance.network_id())
                        .display()
                );
                Ok(())
            }
            Command::Migrate => anyhow::bail!("database migrations are not implemented"),
            Command::Purge => anyhow::bail!("database purge is not implemented"),
        }
    }
}
