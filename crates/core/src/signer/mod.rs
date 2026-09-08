use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::Authorization;
use alloy_network::TxSigner;
use alloy_primitives::{Address, Signature};
use serde::{Deserialize, Serialize};

pub mod simple;

/// A trait representing a key that can sign messages.
#[async_trait::async_trait]
pub trait Signer: Send + Sync {
    /// The registered [`crate::factory::Factory`] tag this signer is rebuilt from.
    fn tag(&self) -> &'static str;

    fn id(&self) -> SignerId;

    /// Returns the address associated with this signer.
    fn address(&self) -> Address;

    /// Signs a message per [EIP-191].
    ///
    /// [EIP-191]: https://eips.ethereum.org/EIPS/eip-191
    async fn personal_sign(&self, message: &[u8]) -> Result<Signature, SignerError>;

    /// Signs structured data per [EIP-712].
    ///
    /// [EIP-712]: https://eips.ethereum.org/EIPS/eip-712
    async fn sign_typed_data(&self, data: &TypedData) -> Result<Signature, SignerError>;

    /// Signs a transaction, deriving the signing hash from `tx` rather than accepting one, so
    /// that an implementation which prompts or refuses can see what it is signing.
    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature, SignerError>;

    /// Signs an [EIP-7702] authorization, delegating the signer's address to the implementation
    /// contract it names.
    ///
    /// [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
    async fn sign_authorization(
        &self,
        authorization: &Authorization,
    ) -> Result<Signature, SignerError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignerId {
    Address(Address),
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait::async_trait]
impl TxSigner<Signature> for dyn Signer {
    fn address(&self) -> Address {
        self.address()
    }

    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature> {
        self.sign_transaction(tx)
            .await
            .map_err(alloy_signer::Error::other)
    }
}

impl std::fmt::Display for SignerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignerId::Address(addr) => write!(f, "addr:{addr:}"),
        }
    }
}
