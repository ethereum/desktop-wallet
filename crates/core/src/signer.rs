use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::Authorization;
use alloy_primitives::{Address, Signature};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignerId {
    Address(Address),
}

/// A trait representing a key that can sign messages. The trait cannot be used to directly
/// access any secret material, and no method that returns it may be added.
#[async_trait::async_trait]
pub trait Signer: Send + Sync {
    /// The registered [`crate::factory::Factory`] tag this signer is rebuilt from.
    fn tag(&self) -> &'static str;

    fn id(&self) -> SignerId;

    /// Signs a message per [EIP-191].
    ///
    /// [EIP-191]: https://eips.ethereum.org/EIPS/eip-191
    async fn personal_sign(&self, message: &[u8]) -> Result<Signature, SignerError>;

    /// Signs structured data per [EIP-712].
    ///
    /// Takes [`TypedData`] rather than a generic `T: SolStruct`, because the types arrive at
    /// runtime in an `eth_signTypedData_v4` request, and because a generic method would not
    /// be callable on a `dyn Signer`.
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
    /// contract it names. Separate from [`Signer::sign_transaction`] because a delegation
    /// changes what the account is rather than what it does once.
    ///
    /// Returns only the signature, so the caller keeps the authorization it built. An
    /// implementation that returned a whole [`alloy_eips::eip7702::SignedAuthorization`]
    /// could substitute the implementation address, chain or nonce it was asked to sign for.
    ///
    /// [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
    async fn sign_authorization(
        &self,
        authorization: &Authorization,
    ) -> Result<Signature, SignerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for SignerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignerId::Address(addr) => write!(f, "addr:{addr:}"),
        }
    }
}
