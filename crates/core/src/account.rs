use alloy_primitives::{Address, Bytes};
use serde::{Deserialize, Serialize};

use crate::{
    asset::AssetId,
    mnemonic::{Mnemonic, MnemonicError},
};

/// The ERC-5564 scheme stealth accounts use: secp256k1 with view tags.
const STEALTH_SCHEME: u32 = 1;

/// One branch of a profile, derived from its recovery phrase at the profile index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: u32,
    pub kind: AccountKind,
    pub label: Option<String>,
    /// Assets enabled for this account on top of its profile's.
    pub assets: Vec<AssetId>,
}

/// Each kind keeps the public side of what it derives, so reading an account never needs the
/// recovery phrase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    /// The address at `m/44'/60'/x'/0/index`. Index 0 is the profile's identity anchor.
    Address { index: u32, address: Address },
    /// The ERC-5564 meta-address: compressed spending and viewing public keys from
    /// `m/44'/60'/x'/5564'/scheme'/{0,1}`.
    Stealth { scheme: u32, meta_address: Bytes },
}

impl AccountRecord {
    /// The address this account holds funds at, if it has a single one.
    #[must_use]
    pub const fn address(&self) -> Option<Address> {
        match &self.kind {
            AccountKind::Address { address, .. } => Some(*address),
            AccountKind::Stealth { .. } => None,
        }
    }
}

impl AccountKind {
    pub fn address(
        mnemonic: &Mnemonic,
        profile_index: u32,
        index: u32,
    ) -> Result<Self, MnemonicError> {
        Ok(Self::Address {
            index,
            address: mnemonic.standard_address(index, profile_index)?,
        })
    }

    pub fn stealth(mnemonic: &Mnemonic, profile_index: u32) -> Result<Self, MnemonicError> {
        let mut meta_address = Vec::with_capacity(66);
        for key in [
            mnemonic.stealth_spending_key(profile_index)?,
            mnemonic.stealth_viewing_key(profile_index)?,
        ] {
            meta_address.extend_from_slice(key.verifying_key().to_encoded_point(true).as_bytes());
        }
        Ok(Self::Stealth {
            scheme: STEALTH_SCHEME,
            meta_address: meta_address.into(),
        })
    }

    /// Whether `self` and `other` derive from the same path, whatever else they hold.
    #[must_use]
    pub fn same_branch(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Address { index: a, .. }, Self::Address { index: b, .. })
            | (Self::Stealth { scheme: a, .. }, Self::Stealth { scheme: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn a_stealth_meta_address_is_two_compressed_public_keys() {
        let mnemonic = Mnemonic::parse(FIXTURE).unwrap();
        let AccountKind::Stealth { meta_address, .. } = AccountKind::stealth(&mnemonic, 0).unwrap()
        else {
            panic!("a stealth account");
        };
        assert_eq!(meta_address.len(), 66);
        assert!(meta_address[..33][0] == 0x02 || meta_address[..33][0] == 0x03);
        assert!(meta_address[33..][0] == 0x02 || meta_address[33..][0] == 0x03);
        assert_ne!(
            AccountKind::stealth(&mnemonic, 1).unwrap(),
            AccountKind::stealth(&mnemonic, 0).unwrap()
        );
    }
}
