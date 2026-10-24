use std::fmt;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    spend::{choose_profile, parse_address},
};

#[derive(Subcommand)]
pub enum Command {
    /// Lists the ERC-20s a profile tracks.
    List(ListArgs),
    /// Tracks an ERC-20, including a default the profile had opted out of.
    OptIn(AddressArgs),
    /// Stops a profile tracking an ERC-20. A default is recorded as opted out; any other token is
    /// removed from its opt-ins.
    OptOut(AddressArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Args, Debug)]
pub struct AddressArgs {
    address: String,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Serialize)]
struct ListedAsset {
    symbol: String,
    address: String,
}

#[derive(Serialize)]
struct ListReport {
    assets: Vec<ListedAsset>,
}

#[derive(Serialize)]
struct ChangeReport {
    action: &'static str,
    symbol: String,
    address: String,
}

impl fmt::Display for ListReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.assets.is_empty() {
            return write!(f, "no assets");
        }
        for (index, asset) in self.assets.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{} {}", asset.symbol, asset.address)?;
        }
        Ok(())
    }
}

impl Report for ListReport {
    const KIND: &'static str = "edw/asset-list";
    const VERSION: u32 = 1;
}

impl fmt::Display for ChangeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.action, self.symbol, self.address)
    }
}

impl Report for ChangeReport {
    const KIND: &'static str = "edw/asset";
    const VERSION: u32 = 1;
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> anyhow::Result<()> {
        match self {
            Self::List(args) => list(global, args).await,
            Self::OptIn(args) => opt_in(global, args).await,
            Self::OptOut(args) => opt_out(global, args).await,
        }
    }
}

async fn list(global: &GlobalArgs, args: &ListArgs) -> anyhow::Result<()> {
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), global.input()).await?;
    let assets = instance.profile_erc20s(&profile).await?;
    ListReport {
        assets: assets
            .into_iter()
            .map(|asset| ListedAsset {
                symbol: asset.symbol,
                address: asset.address.to_string(),
            })
            .collect(),
    }
    .emit(global.mode())
}

async fn opt_in(global: &GlobalArgs, args: &AddressArgs) -> anyhow::Result<()> {
    let address = parse_address(&args.address)?;
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), global.input()).await?;
    let endpoint = instance.rpc().await?;
    let asset = instance
        .opt_in_erc20(&profile, endpoint.as_ref(), address)
        .await?;
    ChangeReport {
        action: "opted in to",
        symbol: asset.symbol,
        address: asset.address.to_string(),
    }
    .emit(global.mode())
}

async fn opt_out(global: &GlobalArgs, args: &AddressArgs) -> anyhow::Result<()> {
    let address = parse_address(&args.address)?;
    let instance = global.open().await?;
    let profile = choose_profile(&instance, args.profile.as_deref(), global.input()).await?;
    let asset = instance.opt_out_erc20(&profile, address).await?;
    ChangeReport {
        action: "opted out of",
        symbol: asset.symbol,
        address: asset.address.to_string(),
    }
    .emit(global.mode())
}
