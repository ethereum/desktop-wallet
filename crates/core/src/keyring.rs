use std::sync::Arc;

use zeroize::Zeroizing;

use crate::{
    database::{Database, scoped::ScopedDatabase},
    mnemonic::{Mnemonic, MnemonicError, MnemonicRecord, db::MnemonicDb},
};

/// The secret material profiles derive from.
///
/// Holds recovery phrases today. Imported private keys and references to external signers
/// belong here as well.
pub struct Keyring {
    db: ScopedDatabase,
}

impl Keyring {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self {
            db: ScopedDatabase::new(store, b"keyring"),
        }
    }

    pub async fn mnemonic(&self, index: u32) -> Result<MnemonicRecord, MnemonicError> {
        self.db
            .get_mnemonic(index)
            .await?
            .ok_or(MnemonicError::Unresolved(index))
    }

    /// The index the next stored phrase will get.
    pub async fn next_index(&self) -> Result<u32, MnemonicError> {
        u32::try_from(self.db.get_mnemonic_indices().await?.len())
            .map_err(|_| MnemonicError::TooMany)
    }

    /// Stores `phrase`. Errors if it is already stored.
    pub async fn add_mnemonic(
        &self,
        phrase: Zeroizing<String>,
    ) -> Result<MnemonicRecord, MnemonicError> {
        let normalized = Mnemonic::parse(&phrase)?.phrase();
        let mut indices = self.db.get_mnemonic_indices().await?;
        for &index in &indices {
            if self.mnemonic(index).await?.phrase == normalized.as_str() {
                return Err(MnemonicError::DuplicatePhrase { index });
            }
        }

        let record = MnemonicRecord {
            index: u32::try_from(indices.len()).map_err(|_| MnemonicError::TooMany)?,
            phrase: normalized.to_string(),
        };
        self.db.put_mnemonic(&record).await?;
        indices.push(record.index);
        self.db.put_mnemonic_indices(&indices).await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory::MemoryDatabase;

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const SECOND_FIXTURE: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    #[tokio::test]
    async fn a_stored_phrase_cannot_be_stored_again() {
        let keyring = Keyring::new(Arc::new(MemoryDatabase::new()));
        keyring
            .add_mnemonic(Zeroizing::new(FIXTURE.to_string()))
            .await
            .unwrap();

        let error = keyring
            .add_mnemonic(Zeroizing::new(FIXTURE.to_string()))
            .await
            .unwrap_err();

        assert!(matches!(error, MnemonicError::DuplicatePhrase { index: 0 }));
    }

    #[tokio::test]
    async fn each_phrase_reads_back_at_its_index() {
        let keyring = Keyring::new(Arc::new(MemoryDatabase::new()));
        for phrase in [FIXTURE, SECOND_FIXTURE] {
            keyring
                .add_mnemonic(Zeroizing::new(phrase.to_string()))
                .await
                .unwrap();
        }

        assert_eq!(keyring.mnemonic(1).await.unwrap().phrase, SECOND_FIXTURE);
        assert_eq!(keyring.next_index().await.unwrap(), 2);
        assert!(matches!(
            keyring.mnemonic(2).await,
            Err(MnemonicError::Unresolved(2))
        ));
    }
}
