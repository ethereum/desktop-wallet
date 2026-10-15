use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::asset::AssetId;

pub mod simple;
pub mod tornadocash;

/// A trait representing a store of assets. Vaults should implement methods for depositing and withdrawing assets.
#[async_trait::async_trait]
pub trait Vault: Send + Sync {
    fn tag(&self) -> &'static str;
    fn id(&self) -> VaultId;
    fn visibility(&self) -> Visibility;

    /// Returns the vault's holdings of the given asset.
    ///
    /// May be called concurrently for different assets.
    async fn balance(&self, asset: &AssetId) -> Result<Balance, VaultError>;
}

/// Whether a vault's holdings can be read from the chain by anyone who knows its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

/// An amount of one asset, split by whether it can be spent yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Balance {
    pub spendable: U256,
    /// Held but not yet spendable, such as a deposit awaiting inclusion or approval.
    pub pending: U256,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum VaultId {
    Address(Address),
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub enum VaultError {
    #[error("unsupported vault id: {0:?}")]
    UnsupportedVaultId(VaultId),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl Balance {
    #[must_use]
    pub const fn spendable(amount: U256) -> Self {
        Self {
            spendable: amount,
            pending: U256::ZERO,
        }
    }

    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            spendable: self.spendable.saturating_add(other.spendable),
            pending: self.pending.saturating_add(other.pending),
        }
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "public"),
            Visibility::Private => write!(f, "private"),
        }
    }
}

impl std::fmt::Display for VaultId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultId::Address(addr) => write!(f, "addr:{addr:}"),
        }
    }
}
