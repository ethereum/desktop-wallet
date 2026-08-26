use std::sync::Arc;

use alloy_dyn_abi::TypedData;
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

use crate::local_signer::db::{LocalSignerDatabaseError, LocalSignerDb};

pub(crate) mod db;

/// A [`Signer`] whose key lives on this machine, loaded through the encrypted [`Database`]
/// the build context supplies.
///
/// The key is held privately and is never returned, logged, or otherwise exported: the only
/// way to use it is to ask this type for a signature (principle 1). That is the whole point
/// of the type, so any method added here that hands back key material would defeat it.
pub struct LocalSigner {
    signer: PrivateKeySigner,
}

#[derive(Debug, thiserror::Error)]
pub enum LocalSignerError {
    #[error("database error: {0}")]
    Database(#[from] LocalSignerDatabaseError),
    #[error("signing error: {0}")]
    Signing(#[from] alloy_signer::Error),
    #[error("typed data error: {0}")]
    TypedData(#[from] alloy_dyn_abi::Error),
}

const LOCAL_SIGNER_TAG: &str = "local-signer";

inventory::submit! {
    Factory::new(LOCAL_SIGNER_TAG, |ctx: BuildContext| {
        Box::pin(async move {
            LocalSigner::from_context(ctx).await.map_err(|e| FactoryError::Other(Box::new(e)))
        })
    })
}

impl LocalSigner {
    /// Creates a signer from `signing_key`, persisting it through `db` so the signer can be
    /// rebuilt later by [`LocalSigner::from_context`].
    ///
    /// `db` is expected to be an encrypted store. This type does not encrypt: it writes
    /// through whatever [`Database`] it is handed, which is why callers should hand it a
    /// [`crate::database::encrypted::EncryptedDatabase`] rather than a bare backend.
    ///
    /// # Errors
    /// Returns an error if the key cannot be written to the database.
    pub async fn new(
        signing_key: SigningKey,
        db: &Arc<dyn Database>,
    ) -> Result<Self, LocalSignerError> {
        db.put_signing_key(&signing_key).await?;
        Ok(Self {
            signer: PrivateKeySigner::from_signing_key(signing_key),
        })
    }

    /// Rebuilds a signer whose key is already in the context's database.
    ///
    /// # Errors
    /// Returns an error if no key is stored, or if the stored bytes are not a valid key.
    pub async fn from_context(ctx: BuildContext) -> Result<Box<dyn Signer>, LocalSignerError> {
        let signing_key = ctx.db.get_signing_key().await?;
        Ok(Box::new(Self {
            signer: PrivateKeySigner::from_signing_key(signing_key),
        }))
    }

    /// The address this signer signs for.
    #[must_use]
    pub fn address(&self) -> Address {
        self.signer.address()
    }
}

#[async_trait::async_trait]
impl Signer for LocalSigner {
    fn tag(&self) -> &'static str {
        LOCAL_SIGNER_TAG
    }

    fn id(&self) -> SignerId {
        SignerId::Address(self.address())
    }

    fn public_key(&self) -> VerifyingKey {
        *self.signer.credential().verifying_key()
    }

    async fn personal_sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        // alloy's `sign_message` is EIP-191 by definition: it hashes with the
        // `\x19Ethereum Signed Message:\n` prefix before signing.
        let signature = self
            .signer
            .sign_message_sync(message)
            .map_err(LocalSignerError::from)?;
        Ok(signature)
    }

    async fn sign_typed_data(&self, data: &TypedData) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_dynamic_typed_data_sync(data)
            .map_err(LocalSignerError::from)?;
        Ok(signature)
    }
}

impl From<LocalSignerError> for SignerError {
    fn from(err: LocalSignerError) -> Self {
        SignerError::Other(Box::new(err))
    }
}
