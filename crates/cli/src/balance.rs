use clap::Args;
use edw_core::asset::AssetId;

use crate::GlobalArgs;

/// Decimals of every network's native asset.
const NATIVE_DECIMALS: u8 = 18;

#[derive(Args, Debug)]
pub struct BalanceArgs {
    /// Profile to show. Shows every profile when omitted.
    profile: Option<String>,
    /// Break each profile down by account.
    #[arg(long)]
    accounts: bool,
}

impl BalanceArgs {
    pub async fn run(&self, global: &GlobalArgs) -> Result<(), anyhow::Error> {
        let instance = global.open().await?;
        let profiles = match &self.profile {
            Some(selector) => vec![instance.profile(selector).await?],
            None => instance.profiles().await?,
        };
        if profiles.is_empty() {
            println!("No profiles. Create one with `edw profile new` or `edw profile import`.");
            return Ok(());
        }

        let endpoint = instance.endpoint(global.rpc_url.as_deref()).await?;
        let native = instance.network().native_asset.clone();
        let assets = instance.assets().await?;
        let describe = |id: AssetId| match id {
            AssetId::Native => (native.clone(), NATIVE_DECIMALS),
            id => assets.iter().find(|asset| asset.id == id).map_or_else(
                || (id.to_string(), 0),
                |asset| (asset.symbol.clone(), asset.decimals),
            ),
        };

        for profile in &profiles {
            let overview = instance.balances(profile, endpoint.as_ref()).await?;
            println!("{}", profile.display_name());

            let mut totals = Vec::new();
            for account in &overview {
                if self.accounts {
                    println!("  account {}  {}", account.account.id, account.address);
                }
                for (id, amount) in &account.balances {
                    if self.accounts {
                        let (symbol, decimals) = describe(*id);
                        println!(
                            "    {} {symbol}",
                            format_units(&amount.to_string(), decimals)
                        );
                    }
                    match totals.iter_mut().find(|(total_id, _)| total_id == id) {
                        Some((_, total)) => *total += *amount,
                        None => totals.push((*id, *amount)),
                    }
                }
            }
            if !self.accounts {
                for (id, total) in totals {
                    let (symbol, decimals) = describe(id);
                    println!("  {} {symbol}", format_units(&total.to_string(), decimals));
                }
            }
        }
        Ok(())
    }
}

/// `amount`, a decimal integer in base units, written with `decimals` fractional digits and
/// trailing zeros dropped.
fn format_units(amount: &str, decimals: u8) -> String {
    let decimals = usize::from(decimals);
    let padded = format!("{amount:0>width$}", width = decimals + 1);
    let (whole, fraction) = padded.split_at(padded.len() - decimals);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_keep_significant_digits_only() {
        assert_eq!(format_units("1500000000000000000", 18), "1.5");
        assert_eq!(format_units("5", 6), "0.000005");
        assert_eq!(format_units("0", 18), "0");
        assert_eq!(format_units("42", 0), "42");
    }
}
