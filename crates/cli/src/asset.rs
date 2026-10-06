use clap::{Args, Subcommand};
use edw_core::asset::AssetId;

use crate::{GlobalArgs, profile::pick_profile};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List the network's configured assets.
    List(AssetListArgs),
    /// Configure an asset, reading its symbol and decimals from the contract.
    Add(AssetAddArgs),
    /// Track an asset for a profile, or for one of its accounts.
    Enable(AssetToggleArgs),
    /// Stop tracking an asset for a profile, or for one of its accounts.
    Disable(AssetToggleArgs),
}

#[derive(Args, Debug)]
pub struct AssetListArgs {}

#[derive(Args, Debug)]
pub struct AssetAddArgs {
    /// `<address>` for an ERC-20, `<address>#<token id>` for one ERC-1155 token.
    asset: AssetId,
}

#[derive(Args, Debug)]
pub struct AssetToggleArgs {
    /// The asset's symbol, or its contract address.
    asset: String,
    /// Profile to change. May be omitted when there is only one.
    #[arg(long)]
    profile: Option<String>,
    /// Change only this account of the profile, by id.
    #[arg(long)]
    account: Option<u32>,
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        match self {
            Self::List(args) => args.run(global).await,
            Self::Add(args) => args.run(global).await,
            Self::Enable(args) => args.run(global, true).await,
            Self::Disable(args) => args.run(global, false).await,
        }
    }
}

impl AssetListArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let assets = global.open().await?.assets().await?;
        if assets.is_empty() {
            println!("No assets. Add one with `edw asset add <address>`.");
        }
        for asset in assets {
            println!(
                "{}  {}  {} decimals",
                asset.symbol, asset.id, asset.decimals
            );
        }
        Ok(())
    }
}

impl AssetAddArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
        let asset = instance.add_asset(self.asset, endpoint.as_ref()).await?;
        println!("Added {} ({} decimals).", asset.symbol, asset.decimals);
        Ok(())
    }
}

impl AssetToggleArgs {
    pub async fn run(&self, global: &GlobalArgs, enable: bool) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let asset = instance.asset(&self.asset).await?;
        let profile = pick_profile(&instance, self.profile.as_deref(), global.input()).await?;
        if enable {
            instance
                .enable_asset(&profile, self.account, asset.id)
                .await?;
        } else {
            instance
                .disable_asset(&profile, self.account, asset.id)
                .await?;
        }

        let verb = if enable { "Enabled" } else { "Disabled" };
        match self.account {
            Some(account) => println!(
                "{verb} {} for account {account} of {}.",
                asset.symbol,
                profile.display_name()
            ),
            None => println!("{verb} {} for {}.", asset.symbol, profile.display_name()),
        }
        Ok(())
    }
}
