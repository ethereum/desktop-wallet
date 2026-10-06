use std::{fmt, str::FromStr};

use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolCall, sol};
use serde::{Deserialize, Serialize};

use crate::network::{NetworkEndpoint, endpoint::NetworkEndpointError};

sol! {
    interface Erc20 {
        function balanceOf(address owner) external view returns (uint256);
        function symbol() external view returns (string);
        function name() external view returns (string);
        function decimals() external view returns (uint8);
    }

    interface Erc1155 {
        function balanceOf(address owner, uint256 id) external view returns (uint256);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetId {
    /// The network's own currency.
    Native,
    Erc20(Address),
    Erc1155 {
        address: Address,
        token_id: U256,
    },
}

/// An asset configured on a network, with the metadata read when it was added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    pub symbol: String,
    pub name: Option<String>,
    pub decimals: u8,
}

#[derive(Debug, thiserror::Error)]
#[error("`{0}` is not an asset; expected `<address>` or `<address>#<token id>`")]
pub struct ParseAssetIdError(String);

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("the native asset is built in: always enabled, and without a contract")]
    Native,
    #[error("{0} does not answer as an ERC-20")]
    NotErc20(Address),
    #[error(transparent)]
    Endpoint(#[from] NetworkEndpointError),
    #[error("unexpected contract response: {0}")]
    Decode(#[from] alloy_sol_types::Error),
}

impl AssetId {
    /// How much of this asset `owner` holds, in base units.
    pub async fn balance_of(
        &self,
        owner: Address,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<U256, AssetError> {
        match self {
            Self::Native => Ok(endpoint.balance(owner).await?),
            Self::Erc20(address) => {
                let call = Erc20::balanceOfCall::new((owner,));
                let returned = endpoint.call_contract(*address, call.abi_encode()).await?;
                Ok(Erc20::balanceOfCall::abi_decode_returns(&returned)?)
            }
            Self::Erc1155 { address, token_id } => {
                let call = Erc1155::balanceOfCall::new((owner, *token_id));
                let returned = endpoint.call_contract(*address, call.abi_encode()).await?;
                Ok(Erc1155::balanceOfCall::abi_decode_returns(&returned)?)
            }
        }
    }

    const fn contract(&self) -> Option<Address> {
        match self {
            Self::Native => None,
            Self::Erc20(address) | Self::Erc1155 { address, .. } => Some(*address),
        }
    }
}

impl Asset {
    /// Reads the metadata of `id` from its contract.
    ///
    /// An ERC-20 must answer `symbol` and `decimals`. ERC-1155 metadata is optional, so a
    /// token without a symbol is named by its token id and has no decimals.
    pub async fn fetch(id: AssetId, endpoint: &dyn NetworkEndpoint) -> Result<Self, AssetError> {
        let address = id.contract().ok_or(AssetError::Native)?;
        let symbol = endpoint
            .call_contract(address, Erc20::symbolCall::new(()).abi_encode())
            .await
            .ok()
            .and_then(|returned| Erc20::symbolCall::abi_decode_returns(&returned).ok());
        let name = endpoint
            .call_contract(address, Erc20::nameCall::new(()).abi_encode())
            .await
            .ok()
            .and_then(|returned| Erc20::nameCall::abi_decode_returns(&returned).ok());

        let (symbol, decimals) = if let AssetId::Erc1155 { token_id, .. } = id {
            (symbol.unwrap_or_else(|| format!("#{token_id}")), 0)
        } else {
            let returned = endpoint
                .call_contract(address, Erc20::decimalsCall::new(()).abi_encode())
                .await?;
            (
                symbol.ok_or(AssetError::NotErc20(address))?,
                Erc20::decimalsCall::abi_decode_returns(&returned)?,
            )
        };
        Ok(Self {
            id,
            symbol,
            name,
            decimals,
        })
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native => f.write_str("native"),
            Self::Erc20(address) => write!(f, "{address}"),
            Self::Erc1155 { address, token_id } => write!(f, "{address}#{token_id}"),
        }
    }
}

/// `<address>` is an ERC-20, `<address>#<token id>` one token of an ERC-1155.
impl FromStr for AssetId {
    type Err = ParseAssetIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseAssetIdError(s.to_string());
        match s.split_once('#') {
            None => Ok(Self::Erc20(s.parse().map_err(|_| invalid())?)),
            Some((address, token_id)) => Ok(Self::Erc1155 {
                address: address.parse().map_err(|_| invalid())?,
                token_id: token_id.parse().map_err(|_| invalid())?,
            }),
        }
    }
}
