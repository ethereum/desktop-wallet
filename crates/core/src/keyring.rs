use std::sync::Arc;

use zeroize::Zeroizing;

use crate::{
    database::{Database, scoped::ScopedDatabase},
    external_signer::{ExternalSigner, ExternalSignerRecord, db::ExternalSignerDb},
    mnemonic::{Mnemonic, MnemonicError, MnemonicRecord, db::MnemonicDb},
};

/// The secret material profiles derive from: recovery phrases and references to external
/// signers, numbered in one index space.
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
        if let Some(record) = self
            .mnemonics()
            .await?
            .into_iter()
            .find(|record| record.index == index)
        {
            return Ok(record);
        }
        match self.external_signer(index).await? {
            Some(_) => Err(MnemonicError::NotAPhrase(index)),
            None => Err(MnemonicError::Unresolved(index)),
        }
    }

    pub async fn external_signer(
        &self,
        index: u32,
    ) -> Result<Option<ExternalSignerRecord>, MnemonicError> {
        Ok(self
            .db
            .get_external_signers()
            .await?
            .into_iter()
            .find(|record| record.index == index))
    }

    /// The index the next stored phrase or signer will get.
    pub async fn next_index(&self) -> Result<u32, MnemonicError> {
        let stored = self.mnemonics().await?.len() + self.db.get_external_signers().await?.len();
        u32::try_from(stored).map_err(|_| MnemonicError::TooMany)
    }

    /// Stores `signer`. Errors if it is already stored.
    pub async fn add_external_signer(
        &self,
        signer: ExternalSigner,
    ) -> Result<ExternalSignerRecord, MnemonicError> {
        let mut records = self.db.get_external_signers().await?;
        if let Some(existing) = records.iter().find(|record| record.signer == signer) {
            return Err(MnemonicError::DuplicateSigner {
                index: existing.index,
            });
        }
        let record = ExternalSignerRecord {
            index: self.next_index().await?,
            signer,
        };
        records.push(record.clone());
        self.db.put_external_signers(&records).await?;
        Ok(record)
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
            index: self.next_index().await?,
            phrase: normalized.to_string(),
        };
        records.push(record.clone());
        self.db.put_mnemonics(&records).await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use super::*;
    use crate::database::memory::MemoryDatabase;

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const LEDGER: ExternalSigner = ExternalSigner::Ledger {
        identity: Address::repeat_byte(1),
    };

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
    async fn a_stored_signer_cannot_be_stored_again() {
        let keyring = Keyring::new(Arc::new(MemoryDatabase::new()));
        keyring.add_external_signer(LEDGER).await.unwrap();

        let error = keyring.add_external_signer(LEDGER).await.unwrap_err();

        assert!(matches!(error, MnemonicError::DuplicateSigner { index: 0 }));
    }

    #[tokio::test]
    async fn a_phrase_stored_after_a_signer_takes_the_next_index() {
        let keyring = Keyring::new(Arc::new(MemoryDatabase::new()));
        keyring.add_external_signer(LEDGER).await.unwrap();

        let record = keyring
            .add_mnemonic(Zeroizing::new(FIXTURE.to_string()))
            .await
            .unwrap();

        assert_eq!(record.index, 1);
    }

    #[tokio::test]
    async fn a_signer_index_is_not_a_phrase() {
        let keyring = Keyring::new(Arc::new(MemoryDatabase::new()));
        keyring.add_external_signer(LEDGER).await.unwrap();

        let error = keyring.mnemonic(0).await.unwrap_err();

        assert!(matches!(error, MnemonicError::NotAPhrase(0)));
    }
}
