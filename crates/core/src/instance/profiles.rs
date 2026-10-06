use zeroize::Zeroizing;

use super::{Instance, InstanceError};
use crate::{
    database::scoped::ScopedDatabaseExt,
    mnemonic::{
        self, MnemonicRecord,
        db::MnemonicDb,
        resolve_mnemonic,
        scan::{EoaScan, scan_standard_eoas},
    },
    network::NetworkEndpoint,
    profile::simple::{ProfileRecord, bootstrap, db::SimpleProfileDb},
};

impl Instance {
    pub async fn profiles(&self) -> Result<Vec<ProfileRecord>, InstanceError> {
        Ok(self
            .store
            .clone()
            .scoped(b"profiles")
            .list_profiles()
            .await?)
    }

    pub async fn mnemonic_indices(&self) -> Result<Vec<u32>, InstanceError> {
        Ok(self
            .mnemonics()
            .await?
            .iter()
            .map(|record| record.index)
            .collect())
    }

    /// Generates a mnemonic and creates exactly one profile on it, at `profile_index`.
    pub async fn generate_profile(
        &self,
        long_seed: bool,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<(MnemonicRecord, ProfileRecord), InstanceError> {
        Ok(
            mnemonic::generate_as_profile(self.store.clone(), long_seed, profile_index, name)
                .await?,
        )
    }

    /// Stores `phrase` and creates exactly one profile on it, at `profile_index`.
    pub async fn import_profile(
        &self,
        phrase: Zeroizing<String>,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<(MnemonicRecord, ProfileRecord), InstanceError> {
        Ok(mnemonic::import_as_profile(self.store.clone(), phrase, profile_index, name).await?)
    }

    /// Creates a profile on stored mnemonic `mnemonic_index`, at `profile_index` or else at
    /// the smallest unused index.
    pub async fn add_profile(
        &self,
        mnemonic_index: u32,
        profile_index: Option<u32>,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        resolve_mnemonic(&self.mnemonics().await?, mnemonic_index)?;
        let profile_index = match profile_index {
            Some(index) => index,
            None => bootstrap::next_profile_index(&self.profiles().await?, mnemonic_index),
        };
        Ok(
            bootstrap::bootstrap_profile(self.store.clone(), mnemonic_index, profile_index, name)
                .await?,
        )
    }

    /// Renames the profile `selector` names, as `mnemonic/profile` or a unique name.
    pub async fn rename_profile(
        &self,
        selector: &str,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        Ok(bootstrap::rename_profile(self.store.clone(), selector, name).await?)
    }

    pub async fn set_profile_name(
        &self,
        mnemonic_index: u32,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        Ok(
            bootstrap::set_profile_name(self.store.clone(), mnemonic_index, profile_index, name)
                .await?,
        )
    }

    /// Scans `profile`'s standard EOAs on `endpoint` for on-chain use.
    pub async fn scan_profile(
        &self,
        profile: &ProfileRecord,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<EoaScan, InstanceError> {
        let mnemonics = self.mnemonics().await?;
        let mnemonic = resolve_mnemonic(&mnemonics, profile.mnemonic_index)?.mnemonic()?;
        Ok(scan_standard_eoas(&mnemonic, profile.profile_index, endpoint).await?)
    }

    async fn mnemonics(&self) -> Result<Vec<MnemonicRecord>, InstanceError> {
        Ok(self
            .store
            .clone()
            .scoped(b"mnemonics")
            .get_mnemonics()
            .await?)
    }
}
