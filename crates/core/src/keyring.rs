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
            db: ScopedDatabase::new(store, b"mnemonics"),
        }
    }

    pub async fn mnemonics(&self) -> Result<Vec<MnemonicRecord>, MnemonicError> {
        Ok(self.db.get_mnemonics().await?)
    }

    pub async fn mnemonic(&self, index: u32) -> Result<MnemonicRecord, MnemonicError> {
        self.mnemonics()
            .await?
            .into_iter()
            .find(|record| record.index == index)
            .ok_or(MnemonicError::Unresolved(index))
    }

    /// The index the next stored phrase will get.
    pub async fn next_index(&self) -> Result<u32, MnemonicError> {
        u32::try_from(self.mnemonics().await?.len()).map_err(|_| MnemonicError::TooMany)
    }

    /// Stores `phrase`. Errors if it is already stored.
    pub async fn add_mnemonic(
        &self,
        phrase: Zeroizing<String>,
    ) -> Result<MnemonicRecord, MnemonicError> {
        let normalized = Mnemonic::parse(&phrase)?.phrase();
        let mut records = self.mnemonics().await?;
        if let Some(existing) = records
            .iter()
            .find(|record| record.phrase == normalized.as_str())
        {
            return Err(MnemonicError::DuplicatePhrase {
                index: existing.index,
            });
        }
        let record = MnemonicRecord {
            index: u32::try_from(records.len()).map_err(|_| MnemonicError::TooMany)?,
            phrase: normalized.to_string(),
        };
        records.push(record.clone());
        self.db.put_mnemonics(&records).await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory::MemoryDatabase;

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

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
}
