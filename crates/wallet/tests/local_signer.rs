//! Known-answer tests for [`LocalSigner`] (EDW-004).
//!
//! The EIP-712 case uses the worked example published in the EIP itself, so the expected
//! hash and signature come from the specification rather than from this implementation. A
//! test that pins whatever the code currently produces would only detect change, not
//! correctness.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, B256, address, b256, hex, keccak256};
use alloy_signer::k256::ecdsa::SigningKey;
use edw_core::{
    database::Database,
    factory::{BuildContext, try_build_signer},
    signer::{Signer, SignerId},
};
use edw_wallet::{database::memory::MemoryDatabase, local_signer::LocalSigner};

/// The EIP-712 example key, and the address it derives to.
const KEY: [u8; 32] = hex!("c85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4");
const SIGNER_ADDRESS: Address = address!("0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826");

/// The `Mail` example from <https://eips.ethereum.org/EIPS/eip-712>.
const EIP712_MAIL: &str = r#"{
  "types": {
    "EIP712Domain": [
      {"name": "name", "type": "string"},
      {"name": "version", "type": "string"},
      {"name": "chainId", "type": "uint256"},
      {"name": "verifyingContract", "type": "address"}
    ],
    "Person": [
      {"name": "name", "type": "string"},
      {"name": "wallet", "type": "address"}
    ],
    "Mail": [
      {"name": "from", "type": "Person"},
      {"name": "to", "type": "Person"},
      {"name": "contents", "type": "string"}
    ]
  },
  "primaryType": "Mail",
  "domain": {
    "name": "Ether Mail",
    "version": "1",
    "chainId": 1,
    "verifyingContract": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"
  },
  "message": {
    "from": {"name": "Cow", "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},
    "to": {"name": "Bob", "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},
    "contents": "Hello, Bob!"
  }
}"#;

/// keccak256 of `"\x19Ethereum Signed Message:\n11Hello, Bob!"`, the EIP-191 digest of the
/// message used below.
const EIP191_DIGEST: B256 =
    b256!("af0a369c7440ada5f06e224551e765ad1acc4ec60aa08944e72415249fa9213e");

/// The signature EIP-712 publishes as the `eth_signTypedData` result for this example.
const EIP712_SIGNATURE: [u8; 65] = hex!(
    "4355c47d63924e8a72e509b65029052eb6c299d53a04e167c5775fd466751c9d\
     07299936d304c153f6443dfa05f40ff007d72911b6f72307f996231605b91562\
     1c"
);

async fn signer() -> LocalSigner {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    let key = SigningKey::from_slice(&KEY).expect("valid key");
    LocalSigner::new(key, &db).await.expect("build signer")
}

#[tokio::test]
async fn identity_is_derived_from_the_key() {
    let signer = signer().await;

    assert_eq!(signer.address(), SIGNER_ADDRESS);
    assert_eq!(signer.id(), SignerId::Address(SIGNER_ADDRESS));
    assert_eq!(
        Address::from_public_key(&signer.public_key()),
        SIGNER_ADDRESS,
        "public key must derive to the signing address"
    );
}

/// EDW-004: `personal_sign` applies the EIP-191 prefix. The expected digest is built here
/// from the raw bytes of the prefix rather than taken from a helper, so the test pins the
/// convention and not just the code path.
#[tokio::test]
async fn personal_sign_uses_the_eip191_prefix() {
    let signer = signer().await;
    let message = b"Hello, Bob!";

    // Built from the prefix's literal bytes rather than from a helper, so the test pins the
    // EIP-191 convention itself, and against a constant so the numeric answer is checkable by
    // hand: keccak256("\x19Ethereum Signed Message:\n11Hello, Bob!").
    let mut prefixed = Vec::new();
    prefixed.extend_from_slice(b"\x19Ethereum Signed Message:\n");
    prefixed.extend_from_slice(message.len().to_string().as_bytes());
    prefixed.extend_from_slice(message);
    let expected_digest = keccak256(&prefixed);
    assert_eq!(expected_digest, EIP191_DIGEST);

    let signature = signer.personal_sign(message).await.expect("sign");

    assert_eq!(
        signature
            .recover_address_from_prehash(&expected_digest)
            .expect("recover"),
        SIGNER_ADDRESS,
        "signature does not verify against the EIP-191 digest"
    );
    // The raw message must NOT be what was signed, or the prefix is not being applied.
    assert_ne!(
        signature
            .recover_address_from_prehash(&keccak256(message))
            .expect("recover"),
        SIGNER_ADDRESS,
        "signature verifies against the unprefixed message: EIP-191 prefix is missing"
    );
}

/// EDW-004: `sign_typed_data` reproduces the worked example published in EIP-712, byte for
/// byte. The expected signature is the `eth_signTypedData` result the EIP itself gives, so
/// this pins the implementation against the specification rather than against itself.
#[tokio::test]
async fn sign_typed_data_matches_the_eip712_published_vector() {
    let signer = signer().await;
    let data: TypedData = serde_json::from_str(EIP712_MAIL).expect("parse typed data");

    let signature = signer.sign_typed_data(&data).await.expect("sign");

    assert_eq!(
        signature.as_bytes(),
        EIP712_SIGNATURE,
        "signature does not match the one published in EIP-712"
    );
    assert_eq!(
        signature
            .recover_address_from_prehash(&data.eip712_signing_hash().expect("signing hash"))
            .expect("recover"),
        SIGNER_ADDRESS
    );
}

/// The key round-trips through the database, so a signer rebuilt by tag signs identically.
#[tokio::test]
async fn a_signer_rebuilt_from_the_database_is_the_same_signer() {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    let key = SigningKey::from_slice(&KEY).expect("valid key");
    let original = LocalSigner::new(key, &db).await.expect("build signer");

    let rebuilt = try_build_signer(original.tag(), BuildContext::new(provider(), db.clone()))
        .await
        .expect("rebuild through the factory");

    assert_eq!(rebuilt.id(), original.id());
    let message = b"Hello, Bob!";
    assert_eq!(
        rebuilt.personal_sign(message).await.expect("sign"),
        original.personal_sign(message).await.expect("sign"),
        "a rebuilt signer must produce identical signatures"
    );
}

/// `BuildContext` carries a provider that a local signer never uses.
fn provider() -> Arc<dyn alloy_provider::Provider> {
    Arc::new(
        alloy_provider::ProviderBuilder::new()
            .connect_http("http://localhost:8545".parse().expect("valid url")),
    )
}
