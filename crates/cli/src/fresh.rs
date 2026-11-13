use std::fmt;

use clap::Args;
use serde::Serialize;

use crate::{
    GlobalArgs,
    report::Report,
    spend::{choose_profile, connect_ledger, mnemonic},
};

#[derive(Args, Debug)]
pub struct NextFreshAddressArgs {
    /// Print the next address without storing it.
    #[arg(long)]
    peek: bool,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Serialize)]
struct FreshReport {
    index: u32,
    path: String,
    address: String,
    stored: bool,
}

impl Report for FreshReport {
    const KIND: &'static str = "edw/next-fresh-address";
    const VERSION: u32 = 1;
}

impl fmt::Display for FreshReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stored = if self.stored { "stored" } else { "not stored" };
        write!(
            f,
            "{}\n{}\n{}\n{stored}",
            self.index, self.path, self.address
        )
    }
}

pub async fn run(global: &GlobalArgs, args: &NextFreshAddressArgs) -> Result<(), anyhow::Error> {
    let instance = global.open().await?;
    let input = global.input();
    let profile = choose_profile(&instance, args.profile.as_deref(), input).await?;
    let endpoint = instance.rpc().await?;
    let fresh = if let Some(ledger) = instance.external_signer(&profile).await? {
        let ledger = &connect_ledger(&ledger, profile.profile_index, 0).await?;
        instance
            .fresh_address_with(&profile, endpoint.as_ref(), |index| async move {
                Ok::<_, anyhow::Error>(
                    ledger
                        .standard_address(profile.profile_index, index)
                        .await?,
                )
            })
            .await?
    } else {
        let phrase = mnemonic(&instance, &profile).await?;
        instance
            .fresh_address(&profile, &phrase, endpoint.as_ref())
            .await?
    };
    let stored = if args.peek {
        false
    } else {
        instance.commit_fresh_address(&profile, fresh.index).await?;
        true
    };
    FreshReport {
        index: fresh.index,
        path: path(profile.profile_index, fresh.index),
        address: fresh.address.to_string(),
        stored,
    }
    .emit(global.mode())
}

fn path(profile_index: u32, index: u32) -> String {
    format!("m/44'/60'/{profile_index}'/0/{index}")
}
