use std::io::IsTerminal;

use clap::Parser;
use edw_cli::Cli;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(cli.log_level().into())
        .from_env_lossy();
    // stdout carries only command output, which `--non-interactive` promises is one JSON document.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    cli.run().await?;

    Ok(())
}
