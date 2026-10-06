//! Tests for [`SimpleSigner`].
#![allow(clippy::expect_used)]

use std::sync::Arc;

use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::Authorization;
use alloy_primitives::{Address, U256, hex};
use alloy_signer::k256::ecdsa::SigningKey;
use alloy_signer_local::PrivateKeySigner;
use edw_core::{
    database::{Database, memory::MemoryDatabase},
    signer::{Signer, simple::SimpleSigner},
};

const KEY: [u8; 32] = hex!("c85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4");
const EIP712_MAIL: &str = include_str!("../fixtures/eip712_mail.json");
const EIP712_SIGNATURE: [u8; 65] = hex!(
    "4355c47d63924e8a72e509b65029052eb6c299d53a04e167c5775fd466751c9d\
     07299936d304c153f6443dfa05f40ff007d72911b6f72307f996231605b91562\
     1c"
);
const MESSAGE: &[u8] = b"Hello, Bob!";
const EIP191_SIGNATURE: [u8; 65] = hex!(
    "d088abb597a29a536423146c15e05a9f18af763823eb041bbb6dea6f6e560f5c\
     45ad634d5594f14191f5f978f7745331fce28c53a348a06ecca512fbc06f65d4\
     1b"
);

async fn signer_from(key: SigningKey) -> SimpleSigner {
    let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
    SimpleSigner::new(key, &db).await.expect("build signer")
}

async fn signer_pair() -> (SimpleSigner, PrivateKeySigner) {
    let reference = PrivateKeySigner::random();
    let ours = signer_from(reference.credential().clone()).await;
    (ours, reference)
}

#[tokio::test]
async fn personal_sign_matches_the_eip191_vector() {
    let signer = signer_from(SigningKey::from_slice(&KEY).expect("valid key")).await;

    let signature = signer.personal_sign(MESSAGE).await.expect("sign");

    assert_eq!(signature.as_bytes(), EIP191_SIGNATURE);
}

#[tokio::test]
async fn sign_typed_data_matches_the_eip712_published_vector() {
    let signer = signer_from(SigningKey::from_slice(&KEY).expect("valid key")).await;
    let data: TypedData = serde_json::from_str(EIP712_MAIL).expect("parse typed data");

    let signature = signer.sign_typed_data(&data).await.expect("sign");

    assert_eq!(signature.as_bytes(), EIP712_SIGNATURE);
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
