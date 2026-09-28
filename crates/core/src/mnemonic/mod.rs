use std::{fmt, str::FromStr, sync::Arc};

use alloy_primitives::Address;
use alloy_signer::k256::ecdsa::SigningKey;
use alloy_signer_local::PrivateKeySigner;
use bip32::{DerivationPath, XPrv};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{
    database::{Database, scoped::ScopedDatabaseExt},
    mnemonic::db::{MnemonicDatabaseError, MnemonicDb},
    profile::simple::{ProfileRecord, bootstrap_profile},
};

pub mod db;
pub mod scan;

const ENGLISH_12: usize = 12;
const ENGLISH_24: usize = 24;

/// A BIP39 English mnemonic (12 or 24 words).
pub struct Mnemonic {
    inner: bip39::Mnemonic,
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct MnemonicRecord {
    pub index: u32,
    pub phrase: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MnemonicError {
    #[error("mnemonic must be 12 or 24 English words")]
    WordCount,
    #[error("invalid mnemonic: {0}")]
    Phrase(#[from] bip39::Error),
    #[error("invalid derivation path: {0}")]
    Path(bip32::Error),
    #[error("key derivation failed: {0}")]
    Derivation(bip32::Error),
    #[error("derived key is not a valid secp256k1 scalar")]
    SigningKey,
    #[error("too many mnemonics")]
    TooMany,
    #[error("no mnemonic {0}")]
    Unresolved(u32),
    #[error(
        "this phrase is already stored as mnemonic {index}; add another profile with `edw profile add --mnemonic {index} --next`"
    )]
    DuplicatePhrase { index: u32 },
    #[error("EOA scan exceeded {0} addresses without an unused gap")]
    ScanLimit(u32),
    #[error(transparent)]
    Database(#[from] MnemonicDatabaseError),
    #[error(transparent)]
    Profile(#[from] crate::profile::simple::ProfileBootstrapError),
    #[error(transparent)]
    ProfileDatabase(#[from] crate::profile::simple::db::SimpleProfileDatabaseError),
    #[error("rpc error: {0}")]
    Rpc(#[from] alloy_transport::TransportError),
}

impl Mnemonic {
    /// Generates a new English mnemonic. 12 words, or 24 when `long_seed` is set.
    pub fn generate(long_seed: bool) -> Result<Self, MnemonicError> {
        let words = if long_seed { ENGLISH_24 } else { ENGLISH_12 };
        let inner = bip39::Mnemonic::generate_in(bip39::Language::English, words)?;
        Ok(Self { inner })
    }

    /// Parses an English 12- or 24-word phrase.
    pub fn parse(phrase: &str) -> Result<Self, MnemonicError> {
        let cleaned = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
        let inner = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, &cleaned)?;
        let words = inner.word_count();
        if words != ENGLISH_12 && words != ENGLISH_24 {
            return Err(MnemonicError::WordCount);
        }
        Ok(Self { inner })
    }

    #[must_use]
    pub fn phrase(&self) -> Zeroizing<String> {
        Zeroizing::new(self.inner.to_string())
    }

    /// Standard public-address key at `m/44'/60'/<profileIndex>'/0/<address_index>`.
    ///
    /// `profile_index` defaults to `0` when omitted (`None`).
    pub fn standard_address_key(
        &self,
        address_index: u32,
        profile_index: impl Into<Option<u32>>,
    ) -> Result<SigningKey, MnemonicError> {
        let profile_index = profile_index.into().unwrap_or(0);
        self.derive(&format!(
            "m/44'/{coin}'/{profile_index}'/0/{address_index}",
            coin = 60
        ))
    }

    /// Standard public address at `m/44'/60'/<profileIndex>'/0/<address_index>`.
    ///
    /// `profile_index` defaults to `0` when omitted (`None`).
    pub fn standard_address(
        &self,
        address_index: u32,
        profile_index: impl Into<Option<u32>>,
    ) -> Result<Address, MnemonicError> {
        let key = self.standard_address_key(address_index, profile_index)?;
        Ok(PrivateKeySigner::from_signing_key(key).address())
    }

    /// Stealth spending key at `m/44'/60'/<profileIndex>'/5564'/1'/0`.
    ///
    /// `profile_index` defaults to `0` when omitted (`None`).
    pub fn stealth_spending_key(
        &self,
        profile_index: impl Into<Option<u32>>,
    ) -> Result<SigningKey, MnemonicError> {
        self.stealth_key(profile_index, 0)
    }

    /// Stealth viewing key at `m/44'/60'/<profileIndex>'/5564'/1'/1`.
    ///
    /// `profile_index` defaults to `0` when omitted (`None`).
    pub fn stealth_viewing_key(
        &self,
        profile_index: impl Into<Option<u32>>,
    ) -> Result<SigningKey, MnemonicError> {
        self.stealth_key(profile_index, 1)
    }

    fn stealth_key(
        &self,
        profile_index: impl Into<Option<u32>>,
        role: u32,
    ) -> Result<SigningKey, MnemonicError> {
        let profile_index = profile_index.into().unwrap_or(0);
        self.derive(&format!(
            "m/44'/{coin}'/{profile_index}'/5564'/1'/{role}",
            coin = 60
        ))
    }

    fn derive(&self, path: &str) -> Result<SigningKey, MnemonicError> {
        let seed = self.inner.to_seed("");
        let path = DerivationPath::from_str(path).map_err(MnemonicError::Path)?;
        let xprv = XPrv::derive_from_path(seed, &path).map_err(MnemonicError::Derivation)?;
        SigningKey::from_slice(&xprv.to_bytes()).map_err(|_| MnemonicError::SigningKey)
    }
}

impl MnemonicRecord {
    pub fn mnemonic(&self) -> Result<Mnemonic, MnemonicError> {
        Mnemonic::parse(&self.phrase)
    }
}

impl fmt::Debug for Mnemonic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mnemonic").field("phrase", &"***").finish()
    }
}

impl fmt::Debug for MnemonicRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MnemonicRecord")
            .field("index", &self.index)
            .field("phrase", &"***")
            .finish()
    }
}

/// Persists a mnemonic. Does not create a profile. Errors if the phrase is already stored.
pub async fn add_mnemonic(
    store: Arc<dyn Database>,
    phrase: Zeroizing<String>,
) -> Result<MnemonicRecord, MnemonicError> {
    let parsed = Mnemonic::parse(&phrase)?;
    let normalized = parsed.phrase();
    let mnemonics_db = store.scoped(b"mnemonics");
    let mut records = mnemonics_db.get_mnemonics().await?;
    if let Some(existing) = records
        .iter()
        .find(|record| record.phrase == normalized.as_str())
    {
        return Err(MnemonicError::DuplicatePhrase {
            index: existing.index,
        });
    }
    let index = u32::try_from(records.len()).map_err(|_| MnemonicError::TooMany)?;
    let record = MnemonicRecord {
        index,
        phrase: normalized.to_string(),
    };
    records.push(record.clone());
    mnemonics_db.put_mnemonics(&records).await?;
    Ok(record)
}

/// Generates and stores the first mnemonic on a new network instance, with profile 0.
pub async fn seed_new_instance(
    store: Arc<dyn Database>,
    long_seed: bool,
) -> Result<MnemonicRecord, MnemonicError> {
    let mnemonic = Mnemonic::generate(long_seed)?;
    let record = add_mnemonic(store.clone(), mnemonic.phrase()).await?;
    bootstrap_profile(store, record.index, 0, None).await?;
    Ok(record)
}

pub async fn load_mnemonics(
    store: Arc<dyn Database>,
) -> Result<Vec<MnemonicRecord>, MnemonicError> {
    Ok(store.scoped(b"mnemonics").get_mnemonics().await?)
}

pub async fn put_mnemonics(
    store: Arc<dyn Database>,
    records: &[MnemonicRecord],
) -> Result<(), MnemonicError> {
    store.scoped(b"mnemonics").put_mnemonics(records).await?;
    Ok(())
}

pub fn resolve_mnemonic(
    records: &[MnemonicRecord],
    index: u32,
) -> Result<&MnemonicRecord, MnemonicError> {
    records
        .iter()
        .find(|record| record.index == index)
        .ok_or(MnemonicError::Unresolved(index))
}

/// Stores a new mnemonic and creates exactly one profile at `profile_index`.
pub async fn import_as_profile(
    store: Arc<dyn Database>,
    phrase: Zeroizing<String>,
    profile_index: u32,
    profile_name: Option<String>,
) -> Result<(MnemonicRecord, ProfileRecord), MnemonicError> {
    let mnemonic = add_mnemonic(store.clone(), phrase).await?;
    let profile = bootstrap_profile(store, mnemonic.index, profile_index, profile_name).await?;
    Ok((mnemonic, profile))
}

/// Generates a mnemonic and creates exactly one profile at `profile_index`.
pub async fn generate_as_profile(
    store: Arc<dyn Database>,
    long_seed: bool,
    profile_index: u32,
    profile_name: Option<String>,
) -> Result<(MnemonicRecord, ProfileRecord), MnemonicError> {
    let generated = Mnemonic::generate(long_seed)?;
    import_as_profile(store, generated.phrase(), profile_index, profile_name).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{database::memory::MemoryDatabase, profile::simple::db::SimpleProfileDb};

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const SECOND_FIXTURE: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    fn fixture() -> Mnemonic {
        Mnemonic::parse(FIXTURE).unwrap()
    }

    fn key_bytes(key: &SigningKey) -> [u8; 32] {
        key.to_bytes().into()
    }

    #[test]
    fn generate_is_12_or_24_english_words() {
        let short = Mnemonic::generate(false).unwrap().phrase();
        assert_eq!(short.split_whitespace().count(), 12);
        Mnemonic::parse(&short).unwrap();

        let long = Mnemonic::generate(true).unwrap().phrase();
        assert_eq!(long.split_whitespace().count(), 24);
        Mnemonic::parse(&long).unwrap();
    }

    #[test]
    fn parse_rejects_wrong_word_count_and_bad_checksum() {
        let fifteen = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert!(matches!(
            Mnemonic::parse(fifteen),
            Err(MnemonicError::WordCount | MnemonicError::Phrase(_))
        ));

        let bad = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        assert!(Mnemonic::parse(bad).is_err());
    }

    #[test]
    fn derivation_is_deterministic_and_path_distinct() {
        let mnemonic = fixture();

        let std_0 = key_bytes(&mnemonic.standard_address_key(0, None).unwrap());
        let std_0_explicit = key_bytes(&mnemonic.standard_address_key(0, Some(0)).unwrap());
        assert_eq!(std_0, std_0_explicit);

        let std_1 = key_bytes(&mnemonic.standard_address_key(1, None).unwrap());
        let profile_1 = key_bytes(&mnemonic.standard_address_key(0, Some(1)).unwrap());
        let spend = key_bytes(&mnemonic.stealth_spending_key(None).unwrap());
        let view = key_bytes(&mnemonic.stealth_viewing_key(None).unwrap());
        let spend_1 = key_bytes(&mnemonic.stealth_spending_key(Some(1)).unwrap());
        let view_1 = key_bytes(&mnemonic.stealth_viewing_key(Some(1)).unwrap());

        let keys = [std_0, std_1, profile_1, spend, view, spend_1, view_1];
        for (i, a) in keys.iter().enumerate() {
            for b in keys.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }

        let again = fixture();
        assert_eq!(
            std_0,
            key_bytes(&again.standard_address_key(0, None).unwrap())
        );
        assert_eq!(spend, key_bytes(&again.stealth_spending_key(None).unwrap()));
        assert_eq!(view, key_bytes(&again.stealth_viewing_key(None).unwrap()));

        // Known-answer: BIP39 `abandon…about` at m/44'/60'/0'/0/0.
        let expected = alloy_primitives::hex::decode_to_array::<_, 32>(
            "1ab42cc412b618bdea3a599e3c9bae199ebf030895b039e9db1e30dafb12b727",
        )
        .unwrap();
        assert_eq!(std_0, expected);
    }

    #[tokio::test]
    async fn mnemonic_db_round_trips_without_exposing_phrase_on_profiles() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let record = add_mnemonic(store.clone(), Zeroizing::new(FIXTURE.to_string()))
            .await
            .unwrap();
        let profile = bootstrap_profile(store.clone(), record.index, 0, None)
            .await
            .unwrap();

        assert_eq!(record.index, 0);
        assert_eq!(record.phrase, FIXTURE);

        let loaded = load_mnemonics(store.clone()).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].phrase, FIXTURE);

        let profiles = store
            .clone()
            .scoped(b"profiles")
            .list_profiles()
            .await
            .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].mnemonic_index, 0);
        assert_eq!(profiles[0].profile_index, 0);
        assert_eq!(profiles[0].display_name(), "default");
        assert!(profiles[0].name.is_none());
        assert_eq!(profile.display_name(), "default");

        let encoded = postcard::to_stdvec(&profiles[0]).unwrap();
        let phrase_bytes = FIXTURE.as_bytes();
        assert!(
            !encoded
                .windows(phrase_bytes.len())
                .any(|w| w == phrase_bytes)
        );
    }

    #[tokio::test]
    async fn add_mnemonic_does_not_create_a_profile() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        add_mnemonic(store.clone(), Zeroizing::new(FIXTURE.to_string()))
            .await
            .unwrap();
        let profiles = store.scoped(b"profiles").list_profiles().await.unwrap();
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn seed_new_instance_creates_default_mnemonic_and_profile() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let record = seed_new_instance(store.clone(), false).await.unwrap();
        assert_eq!(record.index, 0);
        assert_eq!(record.phrase.split_whitespace().count(), 12);

        let profiles = store.scoped(b"profiles").list_profiles().await.unwrap();
        assert_eq!(
            (profiles[0].mnemonic_index, profiles[0].profile_index),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn import_creates_only_the_requested_index() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        seed_new_instance(store.clone(), false).await.unwrap();

        let created = crate::profile::simple::create_next_profile(store.clone(), 0, None)
            .await
            .unwrap();
        assert_eq!(created.profile_index, 1);
        assert_eq!(created.display_name(), "profile #1");

        let imported = import_as_profile(
            store.clone(),
            Zeroizing::new(SECOND_FIXTURE.to_string()),
            3,
            Some("work".into()),
        )
        .await
        .unwrap();
        assert_eq!(imported.0.index, 1);
        assert_eq!(imported.1.profile_index, 3);
        assert_eq!(imported.1.display_name(), "work");

        let profiles = store.scoped(b"profiles").list_profiles().await.unwrap();
        let on_second: Vec<u32> = profiles
            .iter()
            .filter(|profile| profile.mnemonic_index == 1)
            .map(|profile| profile.profile_index)
            .collect();
        assert_eq!(on_second, vec![3]);
    }

    #[tokio::test]
    async fn import_rejects_duplicate_phrase() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        add_mnemonic(store.clone(), Zeroizing::new(FIXTURE.to_string()))
            .await
            .unwrap();
        let error = import_as_profile(
            store,
            Zeroizing::new(FIXTURE.to_string()),
            1,
            Some("work".into()),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, MnemonicError::DuplicatePhrase { index: 0 }));
    }

    #[tokio::test]
    async fn generate_as_profile_skips_index_zero_when_requested() {
        let store: Arc<dyn Database> = Arc::new(MemoryDatabase::new());
        let generated = generate_as_profile(store.clone(), false, 2, Some("cold".into()))
            .await
            .unwrap();
        assert_eq!(generated.0.index, 0);
        assert_eq!(generated.1.profile_index, 2);
        let profiles = store.scoped(b"profiles").list_profiles().await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].profile_index, 2);
    }
}
