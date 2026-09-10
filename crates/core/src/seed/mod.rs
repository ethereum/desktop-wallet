//! BIP-39 mnemonics and BIP-44 derivation for a named profile.
//!
//! A [`SeedRecord`] is the derivation identity: mnemonic + bound chain + `profile_index`.
//! It does not store issued HD addresses or `nextIndex`; those belong with profile metadata.

use std::collections::BTreeMap;

use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_signer_local::MnemonicBuilder;
use coins_bip39::{English, Mnemonic as Bip39Mnemonic};
use zeroize::Zeroizing;

use crate::{
    database::Database,
    network::{NetworkId, SimpleNetworkEndpoint, endpoint::NetworkEndpoint},
    seed::db::SeedDb,
};

pub(crate) mod db;

const SCAN_BATCH: u32 = 5;
const CHANGE: u32 = 0;

/// A BIP-39 mnemonic. Zeroized on drop. Not `Debug`, `Clone`, or `Serialize`.
pub struct Mnemonic {
    phrase: Zeroizing<String>,
}

/// Used HD indexes found by [`scan_used`]. Not persisted by this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanResult {
    pub used: BTreeMap<u32, Address>,
    pub next_index: u32,
}

/// Derivation identity for one named profile. Not `Debug`, `Clone`, or `Serialize`.
pub struct SeedRecord {
    mnemonic: Mnemonic,
    pub network_id: NetworkId,
    pub profile_index: u32,
}

/// How many BIP-39 words to generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordCount {
    Twelve,
    TwentyFour,
}

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("invalid BIP-39 mnemonic")]
    InvalidMnemonic,
    #[error("mnemonic must be 12 or 24 words, got {count}")]
    UnsupportedWordCount { count: usize },
    #[error("failed to generate a mnemonic: {0}")]
    Generate(String),
    #[error("derivation failed: {0}")]
    Derivation(String),
    #[error("no seed is stored")]
    MissingSeed,
    #[error("RPC chain {found} does not match the profile's network {}", expected.0)]
    NetworkMismatch { expected: NetworkId, found: u64 },
    #[error("address index overflow")]
    IndexOverflow,
    #[error(transparent)]
    Database(crate::database::DatabaseError),
    #[error("serialization error: {0}")]
    Serialization(postcard::Error),
    #[error("RPC error: {0}")]
    Rpc(#[from] alloy_transport::TransportError),
}

impl Mnemonic {
    /// Generates a fresh 12- or 24-word mnemonic.
    ///
    /// # Errors
    /// Returns [`SeedError::Generate`] if the wordlist cannot produce a phrase.
    pub fn generate(word_count: WordCount) -> Result<Self, SeedError> {
        let mnemonic =
            Bip39Mnemonic::<English>::new_with_count(&mut rand::rngs::OsRng, word_count.words())
                .map_err(|err| SeedError::Generate(err.to_string()))?;
        Ok(Self {
            phrase: Zeroizing::new(mnemonic.to_phrase()),
        })
    }

    /// Parses a 12- or 24-word BIP-39 phrase. Other lengths are rejected.
    ///
    /// # Errors
    /// Returns [`SeedError::UnsupportedWordCount`] or [`SeedError::InvalidMnemonic`].
    pub fn parse(phrase: &str) -> Result<Self, SeedError> {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        if words.len() != 12 && words.len() != 24 {
            return Err(SeedError::UnsupportedWordCount { count: words.len() });
        }
        let normalized = words.join(" ");
        Bip39Mnemonic::<English>::new_from_phrase(&normalized)
            .map_err(|_| SeedError::InvalidMnemonic)?;
        Ok(Self {
            phrase: Zeroizing::new(normalized),
        })
    }

    /// The space-separated words. For the one-time create print only.
    #[must_use]
    pub fn words(&self) -> &str {
        &self.phrase
    }
}

impl SeedRecord {
    #[must_use]
    pub fn new(mnemonic: Mnemonic, network_id: NetworkId, profile_index: u32) -> Self {
        Self {
            mnemonic,
            network_id,
            profile_index,
        }
    }

    /// The mnemonic words. For the one-time create print only.
    #[must_use]
    pub fn words(&self) -> &str {
        self.mnemonic.words()
    }
}

impl WordCount {
    const fn words(self) -> usize {
        match self {
            Self::Twelve => 12,
            Self::TwentyFour => 24,
        }
    }
}

/// Path: `m/44'/60'/{profile_index}'/0/{j}`. Change is always 0.
///
/// # Errors
/// Returns [`SeedError::Derivation`] if the path or phrase cannot produce a key.
pub fn derive_address(record: &SeedRecord, j: u32) -> Result<Address, SeedError> {
    let path = format!("m/44'/60'/{}'/{CHANGE}/{j}", record.profile_index);
    let signer = MnemonicBuilder::<English>::default()
        .phrase(record.words())
        .derivation_path(&path)
        .map_err(|err| SeedError::Derivation(err.to_string()))?
        .build()
        .map_err(|err| SeedError::Derivation(err.to_string()))?;
    Ok(signer.address())
}

/// Writes the full [`SeedRecord`]. There is no public load of the mnemonic alone.
///
/// # Errors
/// Returns a database or serialization error.
pub async fn store_seed(db: &dyn Database, record: &SeedRecord) -> Result<(), SeedError> {
    db.put_seed(record).await
}

/// Loads the [`SeedRecord`] stored in `db`.
///
/// # Errors
/// [`SeedError::MissingSeed`] if nothing is stored, or parse/database errors.
pub async fn load_seed(db: &dyn Database) -> Result<SeedRecord, SeedError> {
    db.get_seed().await
}

/// Errors if the provider's chain id is not the record's bound `network_id`.
///
/// # Errors
/// [`SeedError::NetworkMismatch`] or an RPC error.
pub async fn assert_network(
    record: &SeedRecord,
    provider: &SimpleNetworkEndpoint,
) -> Result<(), SeedError> {
    let found = provider.network_id().await?;
    if found != record.network_id.0 {
        return Err(SeedError::NetworkMismatch {
            expected: record.network_id,
            found,
        });
    }
    Ok(())
}

/// Scans HD indexes in batches of [`SCAN_BATCH`]. Continues only if the current batch had
/// a used address. Does not read or write the database.
///
/// # Errors
/// Derivation or RPC failure.
pub async fn scan_used(
    record: &SeedRecord,
    provider: &SimpleNetworkEndpoint,
) -> Result<ScanResult, SeedError> {
    let mut used = BTreeMap::new();
    let mut start = 0_u32;
    loop {
        let mut batch_used = false;
        for offset in 0..SCAN_BATCH {
            let index = start.checked_add(offset).ok_or(SeedError::IndexOverflow)?;
            let address = derive_address(record, index)?;
            if is_address_used(provider, address).await? {
                batch_used = true;
                used.insert(index, address);
            }
        }
        if !batch_used {
            break;
        }
        start = start
            .checked_add(SCAN_BATCH)
            .ok_or(SeedError::IndexOverflow)?;
    }

    let next_index = used
        .keys()
        .next_back()
        .copied()
        .map_or(0, |n| n.saturating_add(1));
    Ok(ScanResult { used, next_index })
}

async fn is_address_used(
    provider: &SimpleNetworkEndpoint,
    address: Address,
) -> Result<bool, SeedError> {
    let nonce = provider.provider.get_transaction_count(address).await?;
    let balance = provider.provider.get_balance(address).await?;
    let code = provider.provider.get_code_at(address).await?;
    Ok(nonce > 0 || !balance.is_zero() || !code.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alloy_primitives::{Bytes, U64, U256, address};
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{
        database::{encrypted::EncryptedDatabase, memory::MemoryDatabase},
        test_support::mocked_provider,
    };

    const ANVIL: &str = "test test test test test test test test test test test junk";
    const ANVIL_0: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    fn record(profile_index: u32) -> SeedRecord {
        SeedRecord::new(Mnemonic::parse(ANVIL).unwrap(), NetworkId(1), profile_index)
    }

    fn push_unused(asserter: &Asserter) {
        asserter.push_success(&U64::from(0));
        asserter.push_success(&U256::ZERO);
        asserter.push_success(&Bytes::new());
    }

    #[test]
    fn parse_rejects_empty_and_wrong_length() {
        assert!(matches!(
            Mnemonic::parse(""),
            Err(SeedError::UnsupportedWordCount { count: 0 })
        ));
        assert!(matches!(
            Mnemonic::parse("word ".repeat(15).as_str()),
            Err(SeedError::UnsupportedWordCount { count: 15 })
        ));
    }

    #[test]
    fn parse_rejects_bad_checksum() {
        let bad = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        assert!(matches!(
            Mnemonic::parse(bad),
            Err(SeedError::InvalidMnemonic)
        ));
    }

    #[test]
    fn generate_12_and_24_round_trip() {
        let twelve = Mnemonic::generate(WordCount::Twelve).unwrap();
        assert_eq!(twelve.words().split_whitespace().count(), 12);
        assert_eq!(
            Mnemonic::parse(twelve.words()).unwrap().words(),
            twelve.words()
        );

        let twenty_four = Mnemonic::generate(WordCount::TwentyFour).unwrap();
        assert_eq!(twenty_four.words().split_whitespace().count(), 24);
        assert_eq!(
            Mnemonic::parse(twenty_four.words()).unwrap().words(),
            twenty_four.words()
        );
    }

    #[test]
    fn anvil_mnemonic_index_zero_is_the_known_address() {
        assert_eq!(derive_address(&record(0), 0).unwrap(), ANVIL_0);
    }

    #[test]
    fn profile_index_changes_the_account_leaf() {
        let zero = derive_address(&record(0), 0).unwrap();
        let one = derive_address(&record(1), 0).unwrap();
        assert_ne!(zero, one);
    }

    #[tokio::test]
    async fn load_seed_round_trips_any_profile_index() {
        let db = MemoryDatabase::new();
        let original = SeedRecord::new(Mnemonic::parse(ANVIL).unwrap(), NetworkId(11_155_111), 7);
        store_seed(&db, &original).await.unwrap();
        let loaded = load_seed(&db).await.unwrap();
        assert_eq!(loaded.words(), ANVIL);
        assert_eq!(loaded.network_id, NetworkId(11_155_111));
        assert_eq!(loaded.profile_index, 7);
    }

    #[tokio::test]
    async fn assert_network_rejects_a_mismatched_chain() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));
        let record = SeedRecord::new(Mnemonic::parse(ANVIL).unwrap(), NetworkId(11_155_111), 0);
        assert!(matches!(
            assert_network(&record, &mocked_provider(&asserter)).await,
            Err(SeedError::NetworkMismatch {
                expected: NetworkId(11_155_111),
                found: 1
            })
        ));
    }

    #[tokio::test]
    async fn scan_stops_after_a_clean_batch() {
        let asserter = Asserter::new();
        for _ in 0..SCAN_BATCH {
            push_unused(&asserter);
        }

        let result = scan_used(&record(0), &mocked_provider(&asserter))
            .await
            .unwrap();
        assert_eq!(result.next_index, 0);
        assert!(result.used.is_empty());
    }

    #[tokio::test]
    async fn scan_returns_used_map_and_next_index_without_storing() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(1));
        asserter.push_success(&U256::ZERO);
        asserter.push_success(&Bytes::new());
        for _ in 1..SCAN_BATCH {
            push_unused(&asserter);
        }
        for _ in 0..SCAN_BATCH {
            push_unused(&asserter);
        }

        let result = scan_used(&record(0), &mocked_provider(&asserter))
            .await
            .unwrap();
        assert_eq!(result.next_index, 1);
        assert_eq!(result.used.len(), 1);
        assert_eq!(result.used[&0], ANVIL_0);

        let db = MemoryDatabase::new();
        store_seed(&db, &record(0)).await.unwrap();
        let loaded = load_seed(&db).await.unwrap();
        assert_eq!(loaded.profile_index, 0);
    }

    #[tokio::test]
    async fn scan_treats_balance_or_code_as_used() {
        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&U256::from(1));
        asserter.push_success(&Bytes::new());
        for _ in 1..SCAN_BATCH {
            push_unused(&asserter);
        }
        for _ in 0..SCAN_BATCH {
            push_unused(&asserter);
        }

        assert_eq!(
            scan_used(&record(0), &mocked_provider(&asserter))
                .await
                .unwrap()
                .next_index,
            1
        );

        let asserter = Asserter::new();
        asserter.push_success(&U64::from(0));
        asserter.push_success(&U256::ZERO);
        asserter.push_success(&Bytes::from_static(&[0xef, 0x01]));
        for _ in 1..SCAN_BATCH {
            push_unused(&asserter);
        }
        for _ in 0..SCAN_BATCH {
            push_unused(&asserter);
        }

        assert_eq!(
            scan_used(&record(0), &mocked_provider(&asserter))
                .await
                .unwrap()
                .next_index,
            1
        );
    }

    #[tokio::test]
    async fn store_seed_leaves_no_plaintext_mnemonic_in_the_backend() {
        let backend: Arc<MemoryDatabase> = Arc::new(MemoryDatabase::new());
        let db = EncryptedDatabase::create(backend.clone(), b"password")
            .await
            .unwrap();
        store_seed(&db, &record(0)).await.unwrap();

        for key in backend.keys().unwrap() {
            let value = backend.get(&key).await.unwrap().unwrap();
            let haystack = String::from_utf8_lossy(&value);
            assert!(
                !haystack.contains(ANVIL),
                "backend held the mnemonic in plaintext"
            );
        }
    }
}
