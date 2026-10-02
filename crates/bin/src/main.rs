use clap::Parser;
use edw_cli::Cli;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // Logs are diagnostics, and `--non-interactive` promises stdout carries only the JSON
    // document, so they go to stderr whether or not the flag is set.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    cli.run().await?;

    Ok(())
}
