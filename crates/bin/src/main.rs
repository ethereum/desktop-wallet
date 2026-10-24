use std::{io::IsTerminal, process::ExitCode};

use clap::Parser;
use edw_cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
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

    match cli.run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => cli.fail(&error),
    }
}
