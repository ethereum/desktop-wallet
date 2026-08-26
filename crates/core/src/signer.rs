use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, Signature};
use alloy_signer::k256::ecdsa::VerifyingKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignerId {
    Address(Address),
}

/// A trait representing a key that can sign messages on behalf of one address.
///
/// Implementations may hold the key locally, delegate to hardware, or call a remote service.
/// Whichever it is, the key itself is the implementation's business: **no method here returns
/// secret material, and none may be added that does** (principle 1). Callers get signatures
/// and a public key, never a way to extract or export the private half.
///
/// Only two signing operations are exposed, both of which commit to a domain that cannot be
/// confused with a transaction: [`Signer::personal_sign`] prefixes per EIP-191, and
/// [`Signer::sign_typed_data`] hashes per EIP-712. There is deliberately no method that signs
/// a caller-supplied digest, since that would let anything reaching a `Signer` produce a
/// signature over an arbitrary payload, including a transaction or an EIP-7702 authorization.
#[async_trait::async_trait]
pub trait Signer: Send + Sync {
    /// The registered [`crate::factory::Factory`] tag this signer is rebuilt from.
    fn tag(&self) -> &'static str;

    fn id(&self) -> SignerId;

    /// The public half of the signing key. Safe to expose: an address is derivable from it,
    /// and it is what a caller needs to verify a signature this signer produced.
    fn public_key(&self) -> VerifyingKey;

    /// Signs a message per [EIP-191], prefixing it with `\x19Ethereum Signed Message:\n` and
    /// its length before hashing. This is the operation a dapp requests as `personal_sign`.
    ///
    /// The prefix is what keeps a signed message from being replayable as a transaction, so
    /// implementations must apply it rather than signing the bytes as given.
    ///
    /// [EIP-191]: https://eips.ethereum.org/EIPS/eip-191
    async fn personal_sign(&self, message: &[u8]) -> Result<Signature, SignerError>;

    /// Signs structured data per [EIP-712]. Takes dynamically-typed data, so the types come
    /// from the request rather than from a compile-time Rust type, which is what a dapp's
    /// `eth_signTypedData_v4` call carries.
    ///
    /// [EIP-712]: https://eips.ethereum.org/EIPS/eip-712
    async fn sign_typed_data(&self, data: &TypedData) -> Result<Signature, SignerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl SignerId {
    /// The address this signer signs for.
    #[must_use]
    pub fn address(&self) -> Address {
        match self {
            SignerId::Address(addr) => *addr,
        }
    }
}

impl std::fmt::Display for SignerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignerId::Address(addr) => write!(f, "addr:{addr:}"),
        }
    }
}
