use std::fmt;

use alloy_primitives::Address;
use clap::{Args, Subcommand};
use edw_core::asset::{Erc20Asset, default_erc20s};
use serde::Serialize;

use crate::{
    GlobalArgs, failure,
    profile::ProfileArg,
    report::Report,
    spend::{parse_address, shaped_like_address},
};

#[derive(Subcommand)]
pub enum Command {
    /// Lists the ERC-20s a profile tracks.
    List(ListArgs),
    /// Tracks an ERC-20, or tracks a network default again after `remove`.
    Add(AssetArgs),
    /// Stops tracking an ERC-20, which hides its public balance and its Tornado notes.
    Remove(AssetArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[command(flatten)]
    profile: ProfileArg,
}

#[derive(Args, Debug)]
pub struct AssetArgs {
    /// A symbol or the token's address. A token neither tracked nor a network default needs its
    /// address.
    asset: String,
    #[command(flatten)]
    profile: ProfileArg,
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
    const VERSION: u32 = 2;
}

impl Command {
    pub async fn run(&self, global: &GlobalArgs) -> anyhow::Result<()> {
        match self {
            Self::List(args) => list(global, args).await,
            Self::Add(args) => add(global, args).await,
            Self::Remove(args) => remove(global, args).await,
        }
    }
}

async fn list(global: &GlobalArgs, args: &ListArgs) -> anyhow::Result<()> {
    let instance = global.open().await?;
    let profile = args.profile.resolve(&instance, global.input()).await?;
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

async fn add(global: &GlobalArgs, args: &AssetArgs) -> anyhow::Result<()> {
    let instance = global.open().await?;
    let profile = args.profile.resolve(&instance, global.input()).await?;
    let known: Vec<Erc20Asset> = if shaped_like_address(&args.asset) {
        Vec::new()
    } else {
        instance
            .profile_erc20s(&profile)
            .await?
            .into_iter()
            .chain(default_erc20s(instance.network().id))
            .collect()
    };
    let address = address_of(&args.asset, &known)?.ok_or_else(|| {
        failure::usage(format!(
            "no tracked or default ERC-20 is called {}; pass its address",
            args.asset
        ))
    })?;
    let endpoint = instance.rpc().await?;
    let asset = instance
        .opt_in_erc20(&profile, endpoint.as_ref(), address)
        .await?;
    ChangeReport {
        action: "tracking",
        symbol: asset.symbol,
        address: asset.address.to_string(),
    }
    .emit(global.mode())
}

async fn remove(global: &GlobalArgs, args: &AssetArgs) -> anyhow::Result<()> {
    let instance = global.open().await?;
    let profile = args.profile.resolve(&instance, global.input()).await?;
    let tracked = instance.profile_erc20s(&profile).await?;
    let address = address_of(&args.asset, &tracked)?.ok_or_else(|| {
        failure::usage(format!(
            "no tracked ERC-20 is called {}; see `edw asset list`",
            args.asset
        ))
    })?;
    let asset = instance.opt_out_erc20(&profile, address).await?;
    ChangeReport {
        action: "stopped tracking",
        symbol: asset.symbol,
        address: asset.address.to_string(),
    }
    .emit(global.mode())
}

/// The address `selector` names: parsed when shaped like an address, without consulting `known`,
/// else the one token in `known` with that symbol, or `None` when there is no such token.
///
/// # Errors
///
/// Fails on a malformed address, and when several tokens share the symbol.
pub fn address_of(selector: &str, known: &[Erc20Asset]) -> anyhow::Result<Option<Address>> {
    if shaped_like_address(selector) {
        return parse_address(selector).map(Some);
    }
    let mut matches: Vec<Address> = known
        .iter()
        .filter(|asset| asset.symbol.eq_ignore_ascii_case(selector))
        .map(|asset| asset.address)
        .collect();
    matches.sort_unstable();
    matches.dedup();
    match matches.as_slice() {
        [only] => Ok(Some(*only)),
        [] => Ok(None),
        several => Err(failure::usage(format!(
            "several ERC-20s are called {selector}; pass one address: {}",
            several
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(byte: u8, symbol: &str) -> Erc20Asset {
        Erc20Asset {
            address: Address::repeat_byte(byte),
            symbol: symbol.to_string(),
            decimals: 18,
        }
    }

    #[test]
    fn a_symbol_resolves_only_when_it_names_one_token() {
        let known = [
            token(1, "DAI"),
            token(2, "USDC"),
            token(1, "DAI"),
            token(3, "FOO"),
            token(4, "FOO"),
        ];
        assert_eq!(
            address_of("dai", &known).ok(),
            Some(Some(Address::repeat_byte(1)))
        );
        assert_eq!(address_of("BAR", &known).ok(), Some(None));
        let unknown = Address::repeat_byte(9);
        assert_eq!(
            address_of(&unknown.to_string(), &known).ok(),
            Some(Some(unknown))
        );
        assert!(address_of("FOO", &known).is_err());
    }
}
