use std::{fmt, str::FromStr};

use alloy_primitives::Address;
use alloy_signer::k256::ecdsa::SigningKey;
use alloy_signer_local::PrivateKeySigner;
use bip32::{DerivationPath, XPrv};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::mnemonic::db::MnemonicDatabaseError;

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
    #[error("this recovery phrase is already stored")]
    DuplicatePhrase { index: u32 },
    #[error("EOA scan exceeded {0} addresses without an unused gap")]
    ScanLimit(u32),
    #[error(transparent)]
    Database(#[from] MnemonicDatabaseError),
    #[error("network error: {0}")]
    Network(#[from] crate::network::endpoint::NetworkEndpointError),
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

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
}
