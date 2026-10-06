use zeroize::Zeroizing;

use super::{Instance, InstanceError};
use crate::{
    keyring::Keyring,
    mnemonic::{Mnemonic, MnemonicRecord, scan::EoaScan},
    network::NetworkEndpoint,
    profile::{ProfileIndex, ProfileRecord},
};

impl Instance {
    pub async fn profiles(&self) -> Result<Vec<ProfileRecord>, InstanceError> {
        Ok(self.profile_index().list().await?)
    }

    /// The profile `selector` names, as `mnemonic/profile` or a unique name.
    pub async fn profile(&self, selector: &str) -> Result<ProfileRecord, InstanceError> {
        Ok(self.profile_index().find(selector).await?)
    }

    /// Generates a recovery phrase and creates exactly one profile on it, at `profile_index`.
    pub async fn generate_profile(
        &self,
        long_seed: bool,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<(MnemonicRecord, ProfileRecord), InstanceError> {
        let mnemonic = Mnemonic::generate(long_seed)?;
        self.import_profile(mnemonic.phrase(), profile_index, name)
            .await
    }

    /// Stores `phrase` and creates exactly one profile on it, at `profile_index`.
    ///
    /// Checks the profile name first, so a taken name stores no phrase.
    pub async fn import_profile(
        &self,
        phrase: Zeroizing<String>,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<(MnemonicRecord, ProfileRecord), InstanceError> {
        let keyring = self.keyring();
        let profiles = self.profile_index();
        let candidate = ProfileRecord::new(keyring.next_index().await?, profile_index, name);
        profiles.check_name_free(&candidate).await?;

        let mnemonic = keyring.add_mnemonic(phrase).await?;
        let profile = profiles
            .create(mnemonic.index, Some(profile_index), candidate.name)
            .await?;
        Ok((mnemonic, profile))
    }

    /// Creates a profile on stored mnemonic `mnemonic_index`, at `profile_index` or else at
    /// the smallest unused index.
    pub async fn add_profile(
        &self,
        mnemonic_index: u32,
        profile_index: Option<u32>,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        self.keyring().mnemonic(mnemonic_index).await?;
        Ok(self
            .profile_index()
            .create(mnemonic_index, profile_index, name)
            .await?)
    }

    /// Renames the profile `selector` names, as `mnemonic/profile` or a unique name.
    pub async fn rename_profile(
        &self,
        selector: &str,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        Ok(self.profile_index().rename(selector, name).await?)
    }

    pub async fn set_profile_name(
        &self,
        mnemonic_index: u32,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        Ok(self
            .profile_index()
            .set_name(mnemonic_index, profile_index, name)
            .await?)
    }

    /// Scans `profile`'s standard EOAs on `endpoint` for on-chain use.
    pub async fn scan_profile(
        &self,
        profile: &ProfileRecord,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<EoaScan, InstanceError> {
        let mnemonic = self
            .keyring()
            .mnemonic(profile.mnemonic_index)
            .await?
            .mnemonic()?;
        Ok(mnemonic
            .scan_standard_eoas(profile.profile_index, endpoint)
            .await?)
    }

    fn keyring(&self) -> Keyring {
        Keyring::new(self.store.clone())
    }

    fn profile_index(&self) -> ProfileIndex {
        ProfileIndex::new(self.store.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mnemonic::MnemonicError, network::NetworkId, profile::ProfileError};

    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const SECOND_FIXTURE: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    fn instance() -> Instance {
        Instance::in_memory(NetworkId(1337))
    }

    #[tokio::test]
    async fn new_on_an_empty_instance_is_mnemonic_0_and_profile_0() {
        let instance = instance();
        let (mnemonic, profile) = instance.generate_profile(false, 0, None).await.unwrap();

        assert_eq!(mnemonic.index, 0);
        assert_eq!(mnemonic.phrase.split_whitespace().count(), 12);
        assert_eq!(profile.key(), (0, 0));
    }

    #[tokio::test]
    async fn import_creates_only_the_requested_index() {
        let instance = instance();
        instance.generate_profile(false, 0, None).await.unwrap();

        let (mnemonic, profile) = instance
            .import_profile(
                Zeroizing::new(SECOND_FIXTURE.to_string()),
                3,
                Some("work".into()),
            )
            .await
            .unwrap();

        assert_eq!((mnemonic.index, profile.key()), (1, (1, 3)));
        let on_second: Vec<u32> = instance
            .profiles()
            .await
            .unwrap()
            .iter()
            .filter(|profile| profile.mnemonic_index == 1)
            .map(|profile| profile.profile_index)
            .collect();
        assert_eq!(on_second, vec![3]);
    }

    #[tokio::test]
    async fn import_rejects_a_stored_phrase() {
        let instance = instance();
        instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();

        let error = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 1, Some("work".into()))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            InstanceError::Mnemonic(MnemonicError::DuplicatePhrase { index: 0 })
        ));
    }

    #[tokio::test]
    async fn import_with_a_taken_name_stores_no_phrase() {
        let instance = instance();
        instance.generate_profile(false, 0, None).await.unwrap();

        let error = instance
            .import_profile(Zeroizing::new(SECOND_FIXTURE.to_string()), 0, None)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            InstanceError::Profile(ProfileError::DuplicateName(_))
        ));
        assert!(matches!(
            instance.add_profile(1, None, Some("probe".into())).await,
            Err(InstanceError::Mnemonic(MnemonicError::Unresolved(1)))
        ));
    }

    #[tokio::test]
    async fn profile_records_never_hold_the_phrase() {
        let instance = instance();
        instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();

        let encoded = postcard::to_stdvec(&instance.profiles().await.unwrap()).unwrap();
        assert!(
            !encoded
                .windows(FIXTURE.len())
                .any(|window| window == FIXTURE.as_bytes())
        );
    }
}
