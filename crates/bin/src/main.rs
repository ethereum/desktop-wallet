use std::{io::IsTerminal, process::ExitCode};

use clap::Parser;
use edw_cli::Cli;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(cli.log_level().into())
        .from_env_lossy();
    // The Ledger crates log each device error they also return, which the CLI prints.
    if cli.log_level() < LevelFilter::DEBUG {
        for quiet in ["coins_ledger=off", "alloy_signer_ledger=off"] {
            if let Ok(directive) = quiet.parse() {
                filter = filter.add_directive(directive);
            }
        }
    }
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
