use std::sync::Arc;

use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::{Authorization, SignedAuthorization};
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, Signature};
use alloy_signer::{
    SignerSync,
    k256::ecdsa::{SigningKey, VerifyingKey},
};
use alloy_signer_local::PrivateKeySigner;
use edw_core::{
    database::Database,
    factory::{BuildContext, Factory, FactoryError},
    signer::{Signer, SignerError, SignerId},
};

use crate::simple_signer::db::{SimpleSignerDatabaseError, SimpleSignerDb};

pub(crate) mod db;

/// A [`Signer`] backed by a local private key, persisted through the [`Database`] the build
/// context supplies.
pub struct SimpleSigner {
    signer: PrivateKeySigner,
}

#[derive(Debug, thiserror::Error)]
pub enum SimpleSignerError {
    #[error("database error: {0}")]
    Database(#[from] SimpleSignerDatabaseError),
    #[error("signing error: {0}")]
    Signing(#[from] alloy_signer::Error),
    #[error("typed data error: {0}")]
    TypedData(#[from] alloy_dyn_abi::Error),
}

const SIMPLE_SIGNER_TAG: &str = "simple-signer";

inventory::submit! {
    Factory::new(SIMPLE_SIGNER_TAG, |ctx: BuildContext| {
        Box::pin(async move {
            SimpleSigner::from_context(ctx).await.map_err(|e| FactoryError::Other(Box::new(e)))
        })
    })
}

impl SimpleSigner {
    /// Creates a signer from `signing_key`, persisting it through `db` so the signer can be
    /// rebuilt later by [`SimpleSigner::from_context`].
    ///
    /// # Errors
    /// Returns an error if the key cannot be written to the database.
    pub async fn new(
        signing_key: SigningKey,
        db: &Arc<dyn Database>,
    ) -> Result<Self, SimpleSignerError> {
        db.put_signing_key(&signing_key).await?;
        Ok(Self::from_key(signing_key))
    }

    /// Creates a signer over `signing_key` without writing it anywhere, so it cannot be
    /// rebuilt by [`SimpleSigner::from_context`]. Use [`SimpleSigner::new`] for anything an
    /// executor or vault will outlive.
    #[must_use]
    pub fn from_key(signing_key: SigningKey) -> Self {
        Self {
            signer: PrivateKeySigner::from_signing_key(signing_key),
        }
    }

    /// Rebuilds a signer whose key is already in the context's database.
    ///
    /// # Errors
    /// Returns an error if no key is stored, or if the stored bytes are not a valid key.
    pub async fn from_context(ctx: BuildContext) -> Result<Box<dyn Signer>, SimpleSignerError> {
        let signing_key = ctx.db.get_signing_key().await?;
        Ok(Box::new(Self::from_key(signing_key)))
    }

    #[must_use]
    pub fn address(&self) -> Address {
        self.signer.address()
    }
}

// The trait is async so that hardware and remote signers can implement it. A local key
// needs no I/O, so both methods here call alloy's sync variants directly.
#[async_trait::async_trait]
impl Signer for SimpleSigner {
    fn tag(&self) -> &'static str {
        SIMPLE_SIGNER_TAG
    }

    fn id(&self) -> SignerId {
        SignerId::Address(self.address())
    }

    fn public_key(&self) -> VerifyingKey {
        *self.signer.credential().verifying_key()
    }

    async fn personal_sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_message_sync(message)
            .map_err(SimpleSignerError::from)?;
        Ok(signature)
    }

    async fn sign_typed_data(&self, data: &TypedData) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_dynamic_typed_data_sync(data)
            .map_err(SimpleSignerError::from)?;
        Ok(signature)
    }

    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_transaction_sync(tx)
            .map_err(SimpleSignerError::from)?;
        Ok(signature)
    }

    async fn sign_authorization(
        &self,
        authorization: &Authorization,
    ) -> Result<SignedAuthorization, SignerError> {
        let signature = self
            .signer
            .sign_hash_sync(&authorization.signature_hash())
            .map_err(SimpleSignerError::from)?;
        Ok(authorization.clone().into_signed(signature))
    }
}

impl From<SimpleSignerError> for SignerError {
    fn from(err: SimpleSignerError) -> Self {
        SignerError::Other(Box::new(err))
    }
}
