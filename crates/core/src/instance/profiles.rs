use zeroize::Zeroizing;

use super::{Instance, InstanceError};
use crate::{
    account::{AccountKind, AccountRecord},
    keyring::Keyring,
    mnemonic::{Mnemonic, MnemonicRecord},
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
        let profile = self
            .create_profile(&mnemonic, Some(profile_index), candidate.name)
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
        let mnemonic = self.keyring().mnemonic(mnemonic_index).await?;
        self.create_profile(&mnemonic, profile_index, name).await
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

    pub async fn accounts(
        &self,
        profile: &ProfileRecord,
    ) -> Result<Vec<AccountRecord>, InstanceError> {
        Ok(self.profile_index().accounts(profile).await?)
    }

    /// Scans `profile`'s addresses on `endpoint` and adds the used ones as accounts. Returns
    /// the accounts it added.
    pub async fn discover_accounts(
        &self,
        profile: &ProfileRecord,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<Vec<AccountRecord>, InstanceError> {
        let mnemonic = self
            .keyring()
            .mnemonic(profile.mnemonic_index)
            .await?
            .mnemonic()?;
        let scan = mnemonic
            .scan_standard_eoas(profile.profile_index, endpoint)
            .await?;
        let found = scan
            .addresses
            .into_iter()
            .map(|(index, address)| AccountKind::Address { index, address })
            .collect();
        Ok(self.profile_index().add_accounts(profile, found).await?)
    }

    /// Creates a profile on `mnemonic` with its identity address and stealth account.
    async fn create_profile(
        &self,
        mnemonic: &MnemonicRecord,
        profile_index: Option<u32>,
        name: Option<String>,
    ) -> Result<ProfileRecord, InstanceError> {
        let profiles = self.profile_index();
        let profile = profiles.create(mnemonic.index, profile_index, name).await?;
        let phrase = mnemonic.mnemonic()?;
        profiles
            .add_accounts(
                &profile,
                vec![
                    AccountKind::address(&phrase, profile.profile_index, 0)?,
                    AccountKind::stealth(&phrase, profile.profile_index)?,
                ],
            )
            .await?;
        Ok(profile)
    }

    fn keyring(&self) -> Keyring {
        Keyring::new(self.store.clone())
    }

    pub(super) fn profile_index(&self) -> ProfileIndex {
        ProfileIndex::new(self.store.clone())
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Bytes, U64, U256};
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::{
        mnemonic::MnemonicError, network::NetworkId, profile::ProfileError,
        test_support::mocked_provider,
    };

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

    #[tokio::test]
    async fn a_new_profile_has_its_identity_address_and_a_stealth_account() {
        let instance = instance();
        let (_, profile) = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 2, None)
            .await
            .unwrap();

        let mnemonic = Mnemonic::parse(FIXTURE).unwrap();
        let kinds: Vec<AccountKind> = instance
            .accounts(&profile)
            .await
            .unwrap()
            .into_iter()
            .map(|account| account.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                AccountKind::Address {
                    index: 0,
                    address: mnemonic.standard_address(0, 2).unwrap(),
                },
                AccountKind::stealth(&mnemonic, 2).unwrap(),
            ]
        );
    }

    #[tokio::test]
    async fn discovery_adds_the_used_addresses_the_profile_lacks() {
        let instance = instance();
        let (_, profile) = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();

        let asserter = Asserter::new();
        for index in 0..20 {
            if index == 2 {
                asserter.push_success(&U64::from(1));
            } else {
                asserter.push_success(&U64::from(0));
                asserter.push_success(&Bytes::new());
                asserter.push_success(&U256::ZERO);
            }
        }
        let added = instance
            .discover_accounts(&profile, mocked_provider(&asserter).as_ref())
            .await
            .unwrap();

        let indices: Vec<u32> = added
            .iter()
            .filter_map(|account| match account.kind {
                AccountKind::Address { index, .. } => Some(index),
                AccountKind::Stealth { .. } => None,
            })
            .collect();
        assert_eq!(indices, vec![1, 2]);
        assert_eq!(instance.accounts(&profile).await.unwrap().len(), 4);
    }
}
