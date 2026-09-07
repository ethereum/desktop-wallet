use std::{fs::create_dir_all, path::PathBuf, sync::Arc};

use alloy_provider::ProviderBuilder;
use alloy_signer_local::PrivateKeySigner;
use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use edw_wallet::{
    database::file::FileDatabase, simple_executor::SimpleExecutor, simple_profile::SimpleProfile,
};

#[derive(Parser)]
#[command(name = "edw", about = "Ethereum Desktop Wallet CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    global: GlobalArgs,
}

#[derive(Args)]
struct GlobalArgs {
    #[arg(long, global = true, env = "DATA_DIR", default_value = "./.edw/")]
    data_dir: PathBuf,
    #[arg(
        long,
        global = true,
        env = "RPC_URL",
        default_value = "http://localhost:8545"
    )]
    rpc_url: String,
}

#[derive(Subcommand)]
enum Command {
    /// Creates a new profile with a random executor.
    Create { name: String },
    /// Lists all profiles
    List {},
    /// Lists the balance of a profile
    Balance {},
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    run_command(&cli).await?;
    Ok(())
}

async fn run_command(args: &Cli) -> Result<(), anyhow::Error> {
    match &args.command {
        Command::Create { name } => {
            create(name, &args.global).await?;
        }
        Command::List {} => {
            println!("Listing profiles...");
        }
        Command::Balance {} => {
            println!("Checking balance...");
        }
    }

    Ok(())
}

async fn create(name: &str, global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let profile_path = profile_path(name, &global.data_dir);
    if !profile_path.exists() {
        create_dir_all(&profile_path).context("error creating profile directory")?;
    }

    let provider = Arc::new(ProviderBuilder::new().connect_http(global.rpc_url.parse()?));
    let db = Arc::new(profile_db(name, &global.data_dir));
    let executor_signer = PrivateKeySigner::random();

    SimpleProfile::new(provider, db, |ctx| async move {
        SimpleExecutor::new(executor_signer, ctx.provider, ctx.db).await
    })
    .await?;

    Ok(())
}

fn profile_db(name: &str, data_dir: &PathBuf) -> FileDatabase {
    let db_path = profile_path(name, data_dir).join("db");
    FileDatabase::new(db_path)
}

fn profile_path(name: &str, data_dir: &PathBuf) -> PathBuf {
    data_dir.join(name)
}
