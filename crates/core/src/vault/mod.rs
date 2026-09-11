use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::asset::AssetId;

pub mod simple;
#[cfg(feature = "tornadocash")]
pub mod tornadocash;

/// A trait representing a store of assets. Assets can be deposited into and withdrawn from a vault,
/// and the vault can track the total balance of assets it holds.
#[async_trait::async_trait]
pub trait Vault: Send + Sync {
    fn tag(&self) -> &'static str;
    fn id(&self) -> VaultId;

    /// Returns the total balance of the given asset in the vault.
    async fn balance(&self, asset: &AssetId) -> Result<U256, VaultError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VaultId {
    Address(Address),
    Other { tag: String, id: String },
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub enum VaultError {
    #[error("unsupported vault id: {0:?}")]
    UnsupportedVaultId(VaultId),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for VaultId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultId::Address(addr) => write!(f, "addr:{addr:}"),
            VaultId::Other { tag, id } => write!(f, "{tag}:{id}"),
        }
    }
}
