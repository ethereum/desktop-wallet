use std::fmt;

use clap::Args;
use serde::Serialize;

use crate::{
    GlobalArgs,
    profile::ProfileArg,
    report::Report,
    spend::{connect_signer, mnemonic},
};

#[derive(Args, Debug)]
pub struct ReceiveArgs {
    /// Print the next address without storing it.
    #[arg(long)]
    peek: bool,
    #[command(flatten)]
    profile: ProfileArg,
}

#[derive(Serialize)]
struct FreshReport {
    index: u32,
    path: String,
    address: String,
    stored: bool,
}

impl Report for FreshReport {
    const KIND: &'static str = "edw/receive";
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

pub async fn run(global: &GlobalArgs, args: &ReceiveArgs) -> Result<(), anyhow::Error> {
    let instance = global.open().await?;
    let input = global.input();
    let profile = args.profile.resolve(&instance, input).await?;
    let endpoint = instance.rpc().await?;
    let fresh = if let Some(external) = instance.external_signer(&profile).await? {
        let device = &connect_signer(&external, 0, profile.profile_index).await?;
        eprintln!("Looking for a fresh address...");
        instance
            .fresh_address_with(&profile, endpoint.as_ref(), |index| async move {
                Ok::<_, anyhow::Error>(
                    device
                        .standard_address(index, profile.profile_index)
                        .await?,
                )
            })
            .await?
    } else {
        let phrase = mnemonic(&instance, &profile).await?;
        eprintln!("Looking for a fresh address...");
        instance
            .fresh_address(&profile, &phrase, endpoint.as_ref())
            .await?
    };
    let stored = if args.peek {
        false
    } else {
        instance.commit_fresh_address(&profile, fresh.index).await?;
        instance
            .add_address_vault(&profile, fresh.index, fresh.address)
            .await?;
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
