use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

use crate::signer::ledger::{LedgerAccount, LedgerAccountError};

pub mod db;

/// A keyring entry for keys held outside the wallet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSignerRecord {
    pub index: u32,
    pub signer: ExternalSigner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalSigner {
    /// A Ledger running the Ethereum app, identified by its address at `m/44'/60'/0'/0/0`.
    Ledger { identity: Address },
}

impl ExternalSignerRecord {
    /// Connects to the signer for standard address `address_index` of `profile_index`.
    ///
    /// # Errors
    /// Returns [`LedgerAccountError::WrongDevice`] if the connected Ledger is not this one.
    pub async fn connect(
        &self,
        profile_index: u32,
        address_index: u32,
    ) -> Result<LedgerAccount, LedgerAccountError> {
        let ExternalSigner::Ledger { identity } = self.signer;
        let account = LedgerAccount::connect(profile_index, address_index).await?;
        if account.identity().await? != identity {
            return Err(LedgerAccountError::WrongDevice);
        }
        Ok(account)
    }
}
