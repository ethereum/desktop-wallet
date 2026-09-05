//! Tests for [`SimpleSigner`].
//!
//! Each operation is checked twice: against alloy, which is taken as correct, and for
//! `sign_typed_data` against the worked example published in EIP-712. The published vector is
//! the only check that would still fail if alloy itself were wrong.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::Authorization;
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, U256, address, hex, keccak256};
use alloy_signer::{SignerSync, k256::ecdsa::SigningKey};
use alloy_signer_local::PrivateKeySigner;
use edw_core::{
    database::Database,
    factory::{BuildContext, try_build_signer},
    signer::{Signer, SignerId},
};
use edw_wallet::{database::memory::MemoryDatabase, simple_signer::SimpleSigner};

/// The EIP-712 example key, and the address it derives to.
const KEY: [u8; 32] = hex!("c85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4");
const SIGNER_ADDRESS: Address = address!("0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826");

/// The `Mail` example from <https://eips.ethereum.org/EIPS/eip-712>.
const EIP712_MAIL: &str = include_str!("fixtures/eip712_mail.json");

/// The signature EIP-712 publishes as the `eth_signTypedData` result for that example.
const EIP712_SIGNATURE: [u8; 65] = hex!(
    "4355c47d63924e8a72e509b65029052eb6c299d53a04e167c5775fd466751c9d\
     07299936d304c153f6443dfa05f40ff007d72911b6f72307f996231605b91562\
     1c"
);

const MESSAGE: &[u8] = b"Hello, Bob!";

/// The `eth_sign` result for [`MESSAGE`] under [`KEY`], derived from the [EIP-191] preimage
/// `"\x19Ethereum Signed Message:\n" || len || message` and cross-checked against
/// `cast wallet sign`. It pins the output to the specification rather than to alloy, which
/// the parity tests above take as correct.
///
/// [EIP-191]: https://eips.ethereum.org/EIPS/eip-191
const EIP191_SIGNATURE: [u8; 65] = hex!(
    "d088abb597a29a536423146c15e05a9f18af763823eb041bbb6dea6f6e560f5c\
     45ad634d5594f14191f5f978f7745331fce28c53a348a06ecca512fbc06f65d4\
     1b"
);

async fn signer_from(key: SigningKey) -> SimpleSigner {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    SimpleSigner::new(key, &db).await.expect("build signer")
}

/// A [`SimpleSigner`] and a plain alloy signer holding the same random key.
async fn signer_pair() -> (SimpleSigner, PrivateKeySigner) {
    let reference = PrivateKeySigner::random();
    let ours = signer_from(reference.credential().clone()).await;
    (ours, reference)
}

#[tokio::test]
async fn identity_is_derived_from_the_key() {
    let signer = signer_from(SigningKey::from_slice(&KEY).expect("valid key")).await;

    assert_eq!(signer.address(), SIGNER_ADDRESS);
    assert_eq!(signer.id(), SignerId::Address(SIGNER_ADDRESS));
}

#[tokio::test]
async fn personal_sign_matches_alloy() {
    let (ours, reference) = signer_pair().await;

    assert_eq!(
        ours.personal_sign(MESSAGE).await.expect("sign"),
        reference.sign_message_sync(MESSAGE).expect("sign"),
    );
}

#[tokio::test]
async fn personal_sign_matches_the_eip191_vector() {
    let signer = signer_from(SigningKey::from_slice(&KEY).expect("valid key")).await;

    let signature = signer.personal_sign(MESSAGE).await.expect("sign");

    assert_eq!(signature.as_bytes(), EIP191_SIGNATURE);
}

/// Guards the EIP-191 prefix directly, so a signer that hashed the message unprefixed would
/// fail here even if it were consistent with itself.
#[tokio::test]
async fn personal_sign_does_not_sign_the_unprefixed_message() {
    let (ours, reference) = signer_pair().await;

    let signature = ours.personal_sign(MESSAGE).await.expect("sign");

    assert_ne!(
        signature
            .recover_address_from_prehash(&keccak256(MESSAGE))
            .expect("recover"),
        reference.address(),
        "signature verifies against the unprefixed message: EIP-191 prefix is missing"
    );
}

#[tokio::test]
async fn sign_typed_data_matches_alloy() {
    let (ours, reference) = signer_pair().await;
    let data: TypedData = serde_json::from_str(EIP712_MAIL).expect("parse typed data");

    assert_eq!(
        ours.sign_typed_data(&data).await.expect("sign"),
        reference.sign_dynamic_typed_data_sync(&data).expect("sign"),
    );
}

/// The one hardcoded signature: it pins `sign_typed_data` to the specification rather than to
/// alloy, and is the published `eth_signTypedData` result for the fixture.
#[tokio::test]
async fn sign_typed_data_matches_the_eip712_published_vector() {
    let signer = signer_from(SigningKey::from_slice(&KEY).expect("valid key")).await;
    let data: TypedData = serde_json::from_str(EIP712_MAIL).expect("parse typed data");

    let signature = signer.sign_typed_data(&data).await.expect("sign");

    assert_eq!(signature.as_bytes(), EIP712_SIGNATURE);
}

#[tokio::test]
async fn sign_transaction_matches_alloy() {
    let (ours, reference) = signer_pair().await;
    let mut tx = TxEip1559 {
        chain_id: 1,
        nonce: 7,
        gas_limit: 21_000,
        to: Address::repeat_byte(0x11).into(),
        value: U256::from(1_000),
        ..Default::default()
    };
    let mut same_tx = tx.clone();

    assert_eq!(
        ours.sign_transaction(&mut tx).await.expect("sign"),
        reference.sign_transaction_sync(&mut same_tx).expect("sign"),
    );
}

#[tokio::test]
async fn sign_authorization_recovers_to_the_signer() {
    let (ours, reference) = signer_pair().await;
    let authorization = Authorization {
        chain_id: U256::from(1),
        address: Address::repeat_byte(0xAB),
        nonce: 0,
    };

    let signature = ours
        .sign_authorization(&authorization)
        .await
        .expect("sign authorization");

    assert_eq!(
        signature
            .recover_address_from_prehash(&authorization.signature_hash())
            .expect("recover"),
        reference.address(),
        "authorization does not recover to the signing address"
    );
}

#[tokio::test]
async fn a_signer_rebuilt_from_the_database_is_the_same_signer() {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    let key = SigningKey::from_slice(&KEY).expect("valid key");
    let original = SimpleSigner::new(key, &db).await.expect("build signer");

    let rebuilt = try_build_signer(original.tag(), BuildContext::new(provider(), db.clone()))
        .await
        .expect("rebuild through the factory");

    assert_eq!(rebuilt.id(), original.id());
}

/// `BuildContext` carries a provider that this signer never uses.
fn provider() -> Arc<dyn alloy_provider::Provider> {
    Arc::new(
        alloy_provider::ProviderBuilder::new()
            .connect_http("http://localhost:8545".parse().expect("valid url")),
    )
}
