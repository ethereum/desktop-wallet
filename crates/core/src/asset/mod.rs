//! The assets a wallet reads balances for.
//!
//! A default's metadata is fixed here. A profile tracks every default unless it opts out, and
//! tracks any other ERC-20 it opts into. That other token's metadata is read from the network
//! once, when it is first opted into, and is then registered wallet-wide.
//!
//! Functions take `wallet`, the network scope, and `profile`, the profile's own scope.

use alloy_primitives::{
    Address, Bytes, U256, address,
    utils::{ParseUnits, Unit, UnitsError, format_units, parse_units},
};
use alloy_rpc_types_eth::TransactionRequest;
use alloy_sol_types::{SolCall, sol};
use serde::{Deserialize, Serialize};

use crate::{
    asset::db::{AssetDatabaseError, AssetDb},
    call::Call,
    database::Database,
    network::{NetworkEndpoint, NetworkId, endpoint::NetworkEndpointError},
};

pub mod db;

sol!(
    contract Erc20 {
        function balanceOf(address) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function decimals() external view returns (uint8);
        function symbol() external view returns (string);
    }
);

/// Decimals of the native asset, whose balances are read in wei.
pub const NATIVE_DECIMALS: u8 = 18;
const UNKNOWN_SYMBOL: &str = "UNKNOWN";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetId {
    Native,
    Erc20(Address),
}

/// An ERC-20 with the metadata needed to display its amounts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc20Asset {
    pub address: Address,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseAmountError {
    #[error("amount `{0}` is negative")]
    Negative(String),
    #[error("`{amount}` is not a decimal amount with at most {decimals} fractional digits")]
    Invalid { amount: String, decimals: u8 },
}

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("database error: {0}")]
    Database(#[from] AssetDatabaseError),
    #[error("network error: {0}")]
    Network(#[from] NetworkEndpointError),
    #[error("{0} returned no valid balanceOf(); it may not be an ERC-20")]
    NoBalance(Address),
    #[error("{0} returned no valid decimals(); it may not be an ERC-20")]
    NoDecimals(Address),
    #[error("{0} is opted into but not registered")]
    Unregistered(Address),
    #[error("{0} is not a tracked asset")]
    NotTracked(Address),
    #[error("{0} would refuse this transfer: a simulated transfer() returned false")]
    TransferRefused(Address),
}

impl AssetId {
    #[must_use]
    pub fn native() -> Self {
        AssetId::Native
    }

    #[must_use]
    pub fn erc20(address: Address) -> Self {
        AssetId::Erc20(address)
    }
}

impl Erc20Asset {
    fn known(address: Address, symbol: &str, decimals: u8) -> Self {
        Self {
            address,
            symbol: symbol.to_string(),
            decimals,
        }
    }
}

/// The ERC-20s every profile on `network` reads, in display order.
#[must_use]
pub fn default_erc20s(network: NetworkId) -> Vec<Erc20Asset> {
    match network.0 {
        1 => vec![
            Erc20Asset::known(
                address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
                "USDC",
                6,
            ),
            Erc20Asset::known(
                address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
                "USDT",
                6,
            ),
            Erc20Asset::known(
                address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"),
                "DAI",
                18,
            ),
            Erc20Asset::known(
                address!("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
                "WBTC",
                8,
            ),
            Erc20Asset::known(
                address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
                "WETH",
                18,
            ),
        ],
        11_155_111 => vec![
            Erc20Asset::known(
                address!("0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238"),
                "USDC",
                6,
            ),
            Erc20Asset::known(
                address!("0xFF34B3d4Aee8ddCd6F9AFFFB6Fe49bD371b8a357"),
                "DAI",
                18,
            ),
            Erc20Asset::known(
                address!("0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14"),
                "WETH",
                18,
            ),
        ],
        _ => vec![],
    }
}

/// The ERC-20s a profile reads: the network's defaults, minus those it opted out of, then its
/// opted-in assets, each once.
pub async fn profile_erc20s(
    wallet: &dyn Database,
    profile: &dyn Database,
    network: NetworkId,
) -> Result<Vec<Erc20Asset>, AssetError> {
    let opted_out = profile.get_opted_out().await?;
    let mut assets: Vec<Erc20Asset> = default_erc20s(network)
        .into_iter()
        .filter(|asset| !opted_out.contains(&asset.address))
        .collect();
    let registered = wallet.get_erc20s().await?;

    for address in profile.get_opted_in().await? {
        if opted_out.contains(&address) || assets.iter().any(|asset| asset.address == address) {
            continue;
        }
        let asset = registered
            .iter()
            .find(|asset| asset.address == address)
            .ok_or(AssetError::Unregistered(address))?;
        assets.push(asset.clone());
    }
    Ok(assets)
}

/// Opts the profile into the ERC-20 at `address`.
///
/// A default needs no opt-in record. Opting into one that was opted out clears that opt-out.
pub async fn opt_in(
    wallet: &dyn Database,
    profile: &dyn Database,
    endpoint: &dyn NetworkEndpoint,
    network: NetworkId,
    address: Address,
) -> Result<Erc20Asset, AssetError> {
    if let Some(asset) = default_erc20s(network)
        .into_iter()
        .find(|asset| asset.address == address)
    {
        clear_opt_out(profile, address).await?;
        return Ok(asset);
    }

    let mut registered = wallet.get_erc20s().await?;
    let asset = if let Some(asset) = registered.iter().find(|asset| asset.address == address) {
        asset.clone()
    } else {
        let asset = read_erc20(endpoint, address).await?;
        registered.push(asset.clone());
        wallet.put_erc20s(&registered).await?;
        asset
    };

    let mut opted_in = profile.get_opted_in().await?;
    if !opted_in.contains(&address) {
        opted_in.push(address);
        profile.put_opted_in(&opted_in).await?;
    }
    clear_opt_out(profile, address).await?;
    Ok(asset)
}

/// Stops the profile tracking the ERC-20 at `address` and returns the asset it stopped tracking.
///
/// A default is recorded as opted out, so it stays out until opted back in. An extra opt-in is
/// removed from `optedIn`. An address the profile does not track is an error.
pub async fn opt_out(
    wallet: &dyn Database,
    profile: &dyn Database,
    network: NetworkId,
    address: Address,
) -> Result<Erc20Asset, AssetError> {
    let asset = if let Some(asset) = default_erc20s(network)
        .into_iter()
        .find(|asset| asset.address == address)
    {
        asset
    } else {
        let registered = wallet.get_erc20s().await?;
        registered
            .into_iter()
            .find(|asset| asset.address == address)
            .ok_or(AssetError::NotTracked(address))?
    };

    let is_default = default_erc20s(network)
        .iter()
        .any(|asset| asset.address == address);
    let mut opted_in = profile.get_opted_in().await?;
    if let Some(index) = opted_in.iter().position(|opted| *opted == address) {
        opted_in.remove(index);
        profile.put_opted_in(&opted_in).await?;
    } else if !is_default {
        return Err(AssetError::NotTracked(address));
    }

    if is_default {
        remember_opt_out(profile, address).await?;
    }
    Ok(asset)
}

async fn clear_opt_out(profile: &dyn Database, address: Address) -> Result<(), AssetError> {
    let mut opted_out = profile.get_opted_out().await?;
    let Some(index) = opted_out.iter().position(|opted| *opted == address) else {
        return Ok(());
    };
    opted_out.remove(index);
    profile.put_opted_out(&opted_out).await?;
    Ok(())
}

async fn remember_opt_out(profile: &dyn Database, address: Address) -> Result<(), AssetError> {
    let mut opted_out = profile.get_opted_out().await?;
    if !opted_out.contains(&address) {
        opted_out.push(address);
        profile.put_opted_out(&opted_out).await?;
    }
    Ok(())
}

/// Formats `amount` base units as a decimal string with no trailing fractional zeros, such as `1.5`.
pub fn format_amount(amount: U256, decimals: u8) -> Result<String, UnitsError> {
    let formatted = format_units(amount, decimals)?;
    if !formatted.contains('.') {
        return Ok(formatted);
    }
    Ok(formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string())
}

/// Parses a decimal amount such as `1.5` into base units of an asset with `decimals`.
///
/// # Errors
/// [`ParseAmountError::Negative`] for a negative amount, and [`ParseAmountError::Invalid`] for
/// one that is not a decimal or has more fractional digits than `decimals`.
pub fn parse_amount(amount: &str, decimals: u8) -> Result<U256, ParseAmountError> {
    let invalid = || ParseAmountError::Invalid {
        amount: amount.to_string(),
        decimals,
    };
    let text = amount.trim();
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if whole.is_empty() && fraction.is_empty() || !digits(whole) || !digits(fraction) {
        return Err(invalid());
    }
    if fraction.trim_end_matches('0').len() > usize::from(decimals) {
        return Err(invalid());
    }
    match parse_units(text, decimals).map_err(|_| invalid())? {
        ParseUnits::U256(units) => Ok(units),
        ParseUnits::I256(_) => Err(ParseAmountError::Negative(amount.to_string())),
    }
}

/// A call that moves `amount` base units of the ERC-20 at `token` to `to`.
#[must_use]
pub fn transfer_call(token: Address, to: Address, amount: U256) -> Call {
    let data = Erc20::transferCall::new((to, amount)).abi_encode();
    Call::new(token, Bytes::from(data), U256::ZERO)
}

/// Simulates `transfer` from `from`, and fails when the token returns false instead of
/// reverting.
///
/// Empty return data counts as success, as for tokens whose `transfer` returns nothing.
///
/// # Errors
/// [`AssetError::TransferRefused`] when the token returns false, and [`AssetError::Network`]
/// when the call reverts or the endpoint fails.
pub async fn simulate_transfer(
    endpoint: &dyn NetworkEndpoint,
    from: Address,
    transfer: &Call,
) -> Result<(), AssetError> {
    let returned = endpoint
        .call(
            TransactionRequest::default()
                .from(from)
                .to(transfer.target)
                .input(transfer.data.clone().into()),
        )
        .await?;
    if returned.is_empty() || Erc20::transferCall::abi_decode_returns(&returned).unwrap_or(false) {
        Ok(())
    } else {
        Err(AssetError::TransferRefused(transfer.target))
    }
}

/// `owner`'s balance of the ERC-20 at `token`, in base units.
pub async fn erc20_balance(
    endpoint: &dyn NetworkEndpoint,
    token: Address,
    owner: Address,
) -> Result<U256, AssetError> {
    let data = call(endpoint, token, &Erc20::balanceOfCall::new((owner,))).await?;
    Erc20::balanceOfCall::abi_decode_returns(&data).map_err(|_| AssetError::NoBalance(token))
}

async fn read_erc20(
    endpoint: &dyn NetworkEndpoint,
    address: Address,
) -> Result<Erc20Asset, AssetError> {
    let decimals = call(endpoint, address, &Erc20::decimalsCall {}).await?;
    let decimals = Erc20::decimalsCall::abi_decode_returns(&decimals)
        .ok()
        .filter(|decimals| *decimals <= Unit::MAX.get())
        .ok_or(AssetError::NoDecimals(address))?;

    // Only an undecodable symbol falls back. The result is stored, so a transient network
    // failure must not register UNKNOWN for good.
    let symbol = call(endpoint, address, &Erc20::symbolCall {}).await?;
    let symbol = Erc20::symbolCall::abi_decode_returns(&symbol)
        .unwrap_or_else(|_| UNKNOWN_SYMBOL.to_string());

    Ok(Erc20Asset {
        address,
        symbol,
        decimals,
    })
}

async fn call<C: SolCall>(
    endpoint: &dyn NetworkEndpoint,
    token: Address,
    function: &C,
) -> Result<alloy_primitives::Bytes, NetworkEndpointError> {
    endpoint
        .call(
            TransactionRequest::default()
                .to(token)
                .input(function.abi_encode().into()),
        )
        .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use alloy_primitives::{B256, Bytes};
    use alloy_sol_types::SolValue;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{database::memory::MemoryDatabase, test_support::mocked_provider};

    const MAINNET: NetworkId = NetworkId(1);
    const TOKEN: Address = address!("0x1111111111111111111111111111111111111111");

    fn usdc() -> Address {
        default_erc20s(MAINNET)[0].address
    }

    fn respond(asserter: &Asserter, returns: &[u8]) {
        asserter.push_success(&Bytes::copy_from_slice(returns));
    }

    #[test]
    fn a_negative_amount_is_refused_rather_than_wrapped() {
        for amount in ["-1", "-0.1", "-0"] {
            assert!(
                matches!(parse_amount(amount, 18), Err(ParseAmountError::Negative(_))),
                "{amount} must not parse"
            );
        }
    }

    #[test]
    fn an_amount_is_read_in_the_asset_decimals() {
        assert_eq!(parse_amount("1.5", 6).unwrap(), U256::from(1_500_000));
        assert_eq!(
            parse_amount("1.50000000", 6).unwrap(),
            U256::from(1_500_000)
        );
    }

    #[test]
    fn digit_separators_are_refused_rather_than_misread() {
        for amount in ["1.0_1", "1_000"] {
            assert!(parse_amount(amount, 6).is_err(), "{amount} must not parse");
        }
    }

    #[test]
    fn digits_past_the_asset_decimals_are_refused_rather_than_truncated() {
        assert!(parse_amount("1.0000001", 6).is_err());
    }

    fn transfer() -> Call {
        transfer_call(usdc(), Address::repeat_byte(0x22), U256::from(1))
    }

    #[tokio::test]
    async fn a_token_that_returns_false_is_refused() {
        let asserter = Asserter::new();
        respond(&asserter, &false.abi_encode());
        let refused = simulate_transfer(
            mocked_provider(&asserter).as_ref(),
            Address::ZERO,
            &transfer(),
        )
        .await;
        assert!(matches!(refused, Err(AssetError::TransferRefused(_))));
    }

    #[tokio::test]
    async fn a_token_that_returns_true_is_accepted() {
        let asserter = Asserter::new();
        respond(&asserter, &true.abi_encode());
        simulate_transfer(
            mocked_provider(&asserter).as_ref(),
            Address::ZERO,
            &transfer(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_token_whose_transfer_returns_nothing_is_accepted() {
        let asserter = Asserter::new();
        respond(&asserter, &[]);
        simulate_transfer(
            mocked_provider(&asserter).as_ref(),
            Address::ZERO,
            &transfer(),
        )
        .await
        .unwrap();
    }

    #[test]
    fn format_amount_trims_only_fractional_zeros() {
        let cases = [
            (U256::from(1_500_000), 6, "1.5"),
            (U256::from(10).pow(U256::from(18)), 18, "1"),
            (U256::from(100), 0, "100"),
        ];
        for (amount, decimals, expected) in cases {
            assert_eq!(format_amount(amount, decimals).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn a_balance_that_does_not_decode_is_no_balance() {
        let asserter = Asserter::new();
        respond(&asserter, &[]);
        let endpoint = mocked_provider(&asserter);

        let result = erc20_balance(endpoint.as_ref(), TOKEN, Address::ZERO).await;

        assert!(matches!(result, Err(AssetError::NoBalance(TOKEN))));
    }

    #[tokio::test]
    async fn opting_into_a_default_touches_neither_the_network_nor_the_store() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let endpoint = mocked_provider(&Asserter::new());

        opt_in(&wallet, &profile, endpoint.as_ref(), MAINNET, usdc())
            .await
            .unwrap();

        assert!(wallet.get_erc20s().await.unwrap().is_empty());
        assert!(profile.get_opted_in().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn metadata_is_read_once_per_wallet() {
        let wallet = MemoryDatabase::new();
        let asserter = Asserter::new();
        respond(&asserter, &Erc20::decimalsCall::abi_encode_returns(&8));
        respond(
            &asserter,
            &Erc20::symbolCall::abi_encode_returns(&"WBTC".to_string()),
        );
        let endpoint = mocked_provider(&asserter);

        for profile in [MemoryDatabase::new(), MemoryDatabase::new()] {
            let asset = opt_in(&wallet, &profile, endpoint.as_ref(), MAINNET, TOKEN)
                .await
                .unwrap();
            assert_eq!((asset.symbol.as_str(), asset.decimals), ("WBTC", 8));
        }
    }

    #[tokio::test]
    async fn an_undecodable_symbol_registers_as_unknown() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let asserter = Asserter::new();
        respond(&asserter, &Erc20::decimalsCall::abi_encode_returns(&18));
        respond(&asserter, B256::left_padding_from(b"MKR").as_slice());
        let endpoint = mocked_provider(&asserter);

        let asset = opt_in(&wallet, &profile, endpoint.as_ref(), MAINNET, TOKEN)
            .await
            .unwrap();

        assert_eq!(asset.symbol, UNKNOWN_SYMBOL);
    }

    #[tokio::test]
    async fn a_failed_symbol_read_registers_nothing() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let asserter = Asserter::new();
        respond(&asserter, &Erc20::decimalsCall::abi_encode_returns(&18));
        asserter.push_failure_msg("unavailable");
        let endpoint = mocked_provider(&asserter);

        let result = opt_in(&wallet, &profile, endpoint.as_ref(), MAINNET, TOKEN).await;

        assert!(matches!(result, Err(AssetError::Network(_))));
        assert!(wallet.get_erc20s().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_address_without_displayable_decimals_is_refused_and_not_opted_into() {
        for reply in [vec![], Erc20::decimalsCall::abi_encode_returns(&78)] {
            let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
            let asserter = Asserter::new();
            respond(&asserter, &reply);
            let endpoint = mocked_provider(&asserter);

            let result = opt_in(&wallet, &profile, endpoint.as_ref(), MAINNET, TOKEN).await;

            assert!(matches!(result, Err(AssetError::NoDecimals(TOKEN))));
            assert!(profile.get_opted_in().await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn a_profile_reads_the_defaults_then_its_opt_ins_each_once() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let extra = Erc20Asset::known(TOKEN, "EXTRA", 8);
        wallet
            .put_erc20s(std::slice::from_ref(&extra))
            .await
            .unwrap();
        profile.put_opted_in(&[usdc(), TOKEN]).await.unwrap();

        let symbols: Vec<String> = profile_erc20s(&wallet, &profile, MAINNET)
            .await
            .unwrap()
            .into_iter()
            .map(|asset| asset.symbol)
            .collect();

        assert_eq!(symbols, ["USDC", "USDT", "DAI", "WBTC", "WETH", "EXTRA"]);
    }

    #[tokio::test]
    async fn opting_out_of_a_default_hides_it_until_it_is_opted_back_in() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let dai = default_erc20s(MAINNET)
            .into_iter()
            .find(|asset| asset.symbol == "DAI")
            .unwrap()
            .address;

        opt_out(&wallet, &profile, MAINNET, dai).await.unwrap();
        let hidden: Vec<String> = profile_erc20s(&wallet, &profile, MAINNET)
            .await
            .unwrap()
            .into_iter()
            .map(|asset| asset.symbol)
            .collect();
        assert!(!hidden.iter().any(|symbol| symbol == "DAI"));
        assert_eq!(profile.get_opted_out().await.unwrap(), vec![dai]);

        opt_in(
            &wallet,
            &profile,
            mocked_provider(&Asserter::new()).as_ref(),
            MAINNET,
            dai,
        )
        .await
        .unwrap();
        let shown: Vec<String> = profile_erc20s(&wallet, &profile, MAINNET)
            .await
            .unwrap()
            .into_iter()
            .map(|asset| asset.symbol)
            .collect();
        assert!(shown.iter().any(|symbol| symbol == "DAI"));
        assert!(profile.get_opted_out().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn opting_out_of_an_extra_token_removes_it_from_opted_in() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let extra = Erc20Asset::known(TOKEN, "EXTRA", 8);
        wallet
            .put_erc20s(std::slice::from_ref(&extra))
            .await
            .unwrap();
        profile.put_opted_in(&[TOKEN]).await.unwrap();

        opt_out(&wallet, &profile, MAINNET, TOKEN).await.unwrap();

        assert!(profile.get_opted_in().await.unwrap().is_empty());
        assert!(profile.get_opted_out().await.unwrap().is_empty());
        let symbols: Vec<String> = profile_erc20s(&wallet, &profile, MAINNET)
            .await
            .unwrap()
            .into_iter()
            .map(|asset| asset.symbol)
            .collect();
        assert!(!symbols.iter().any(|symbol| symbol == "EXTRA"));
    }

    #[tokio::test]
    async fn opting_out_of_an_untracked_token_is_refused() {
        let (wallet, profile) = (MemoryDatabase::new(), MemoryDatabase::new());
        let result = opt_out(&wallet, &profile, MAINNET, TOKEN).await;
        assert!(matches!(result, Err(AssetError::NotTracked(TOKEN))));
    }
}
