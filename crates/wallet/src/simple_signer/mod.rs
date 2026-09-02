use std::sync::Arc;

use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::Authorization;
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, Signature};
use alloy_signer::{SignerSync, k256::ecdsa::SigningKey};
use alloy_signer_local::PrivateKeySigner;
use edw_core::{
    database::Database,
    factory::{BuildContext, Factory, FactoryError, try_build_signer},
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

pub(crate) const SIMPLE_SIGNER_TAG: &str = "simple-signer";

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

/// Rebuilds `signer` from `ctx.db` through the factory, and records its tag once that has
/// succeeded, so a signer that cannot be recovered leaves no tag behind for a later
/// [`try_build_signer`] to trip over.
///
/// Callers that submit on-chain authorization must do this first: otherwise a
/// [`SimpleSigner::from_key`] (or any signer that has not stored what it needs)
/// can land a 7702 delegation that cannot be recovered after restart.
pub(crate) async fn persist_and_rebuild(
    signer: &dyn Signer,
    ctx: BuildContext,
) -> Result<(), FactoryError> {
    let rebuilt = try_build_signer(signer.tag(), ctx.clone()).await?;
    if rebuilt.id() != signer.id() {
        return Err(FactoryError::Other(Box::new(SignerMismatch {
            persisted: rebuilt.id().to_string(),
            given: signer.id().to_string(),
        })));
    }
    ctx.db
        .put_signer_tag(signer.tag())
        .await
        .map_err(|e| FactoryError::Other(Box::new(e)))?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("persisted signer {persisted} does not match {given}")]
struct SignerMismatch {
    persisted: String,
    given: String,
}

// The trait is async so that hardware and remote signers can implement it. A local key
// needs no I/O, so every method here calls alloy's sync variant directly.
#[async_trait::async_trait]
impl Signer for SimpleSigner {
    fn tag(&self) -> &'static str {
        SIMPLE_SIGNER_TAG
    }

    fn id(&self) -> SignerId {
        SignerId::Address(self.address())
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
    ) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_hash_sync(&authorization.signature_hash())
            .map_err(SimpleSignerError::from)?;
        Ok(signature)
    }
}

impl From<SimpleSignerError> for SignerError {
    fn from(err: SimpleSignerError) -> Self {
        SignerError::Other(Box::new(err))
    }
}

#[cfg(test)]
mod tests {
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{database::memory::MemoryDatabase, test_support::mocked_provider};

    /// Building a [`SimpleSigner`] needs no chain, so the queue is never drawn on.
    fn provider() -> Arc<dyn alloy_provider::Provider> {
        mocked_provider(&Asserter::new())
    }

    #[tokio::test]
    async fn persist_and_rebuild_round_trips_through_the_stored_tag() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let original = SimpleSigner::new(PrivateKeySigner::random().credential().clone(), &db)
            .await
            .expect("build signer");

        persist_and_rebuild(&original, BuildContext::new(provider(), db.clone()))
            .await
            .expect("persist");

        let tag = db.get_signer_tag().await.expect("load tag");
        let rebuilt = try_build_signer(&tag, BuildContext::new(provider(), db))
            .await
            .expect("rebuild from stored tag");
        assert_eq!(rebuilt.id(), original.id());
    }

    #[tokio::test]
    async fn persist_and_rebuild_rejects_an_ephemeral_signer_without_writing_a_tag() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let signer = SimpleSigner::from_key(PrivateKeySigner::random().credential().clone());

        persist_and_rebuild(&signer, BuildContext::new(provider(), db.clone()))
            .await
            .expect_err("from_key has nothing in the database");

        db.get_signer_tag()
            .await
            .expect_err("a tag naming a signer that cannot be rebuilt must not be stored");
    }

    #[tokio::test]
    async fn persist_and_rebuild_rejects_a_signer_that_is_not_the_one_in_the_database() {
        let db: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        SimpleSigner::new(PrivateKeySigner::random().credential().clone(), &db)
            .await
            .expect("build signer");
        let other = SimpleSigner::from_key(PrivateKeySigner::random().credential().clone());

        persist_and_rebuild(&other, BuildContext::new(provider(), db))
            .await
            .expect_err("the stored key rebuilds to a different address");
    }
}
