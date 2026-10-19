use alloy_primitives::Address;
use zeroize::Zeroizing;

use super::{Instance, InstanceError, NETWORK_SCOPE};
use crate::{
    asset::{self, Erc20Asset},
    database::scoped::ScopedDatabase,
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

    /// The ERC-20s `profile` reads: the network's defaults, minus opt-outs, then its opt-ins.
    pub async fn profile_erc20s(
        &self,
        profile: &ProfileRecord,
    ) -> Result<Vec<Erc20Asset>, InstanceError> {
        let wallet = ScopedDatabase::new(self.store.clone(), NETWORK_SCOPE);
        Ok(asset::profile_erc20s(&wallet, &self.profile_db(profile), self.network.id).await?)
    }

    /// Opts `profile` into the ERC-20 at `address`, clearing an opt-out when `address` is a default.
    pub async fn opt_in_erc20(
        &self,
        profile: &ProfileRecord,
        endpoint: &dyn NetworkEndpoint,
        address: Address,
    ) -> Result<Erc20Asset, InstanceError> {
        let wallet = ScopedDatabase::new(self.store.clone(), NETWORK_SCOPE);
        Ok(asset::opt_in(
            &wallet,
            &self.profile_db(profile),
            endpoint,
            self.network.id,
            address,
        )
        .await?)
    }

    /// Stops `profile` tracking the ERC-20 at `address`.
    ///
    /// A default is recorded in `optedOut`. Any other tracked token is removed from `optedIn`.
    pub async fn opt_out_erc20(
        &self,
        profile: &ProfileRecord,
        address: Address,
    ) -> Result<Erc20Asset, InstanceError> {
        let wallet = ScopedDatabase::new(self.store.clone(), NETWORK_SCOPE);
        Ok(asset::opt_out(&wallet, &self.profile_db(profile), self.network.id, address).await?)
    }

    /// The recovery phrase `profile` derives from.
    pub async fn recovery_phrase(
        &self,
        profile: &ProfileRecord,
    ) -> Result<Zeroizing<String>, InstanceError> {
        let record = self.keyring().mnemonic(profile.mnemonic_index).await?;
        Ok(Zeroizing::new(record.phrase.clone()))
    }

    /// Generates a recovery phrase and creates exactly one profile on it, at `profile_index`.
    pub async fn generate_profile(
        &self,
        long_phrase: bool,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<(MnemonicRecord, ProfileRecord), InstanceError> {
        let mnemonic = Mnemonic::generate(long_phrase)?;
        self.import_profile(mnemonic.phrase(), profile_index, name)
            .await
    }

    /// Stores `phrase` and creates exactly one profile on it, at `profile_index`.
    ///
    /// Checks the profile name and index first, so a profile that cannot be created stores no
    /// phrase.
    pub async fn import_profile(
        &self,
        phrase: Zeroizing<String>,
        profile_index: u32,
        name: Option<String>,
    ) -> Result<(MnemonicRecord, ProfileRecord), InstanceError> {
        let keyring = self.keyring();
        let profiles = self.profile_index();
        let candidate = ProfileRecord::new(keyring.next_index().await?, profile_index, name);
        profiles.check_can_create(&candidate).await?;

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
        Ok(self
            .mnemonic(profile)
            .await?
            .scan_standard_eoas(profile.profile_index, endpoint)
            .await?)
    }

    /// `profile`'s standard address at `m/44'/60'/x'/0/index`.
    pub(super) async fn standard_address(
        &self,
        profile: &ProfileRecord,
        index: u32,
    ) -> Result<Address, InstanceError> {
        Ok(self
            .mnemonic(profile)
            .await?
            .standard_address(index, profile.profile_index)?)
    }

    async fn mnemonic(&self, profile: &ProfileRecord) -> Result<Mnemonic, InstanceError> {
        Ok(self
            .keyring()
            .mnemonic(profile.mnemonic_index)
            .await?
            .mnemonic()?)
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
    async fn import_with_an_index_out_of_range_stores_no_phrase() {
        let instance = instance();

        let error = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 1 << 31, None)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            InstanceError::Profile(ProfileError::IndexOutOfRange(_))
        ));
        assert!(matches!(
            instance.add_profile(0, None, None).await,
            Err(InstanceError::Mnemonic(MnemonicError::Unresolved(0)))
        ));
    }

    #[tokio::test]
    async fn recovery_phrase_is_the_profiles_own() {
        let instance = instance();
        instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();
        let (_, second) = instance
            .import_profile(
                Zeroizing::new(SECOND_FIXTURE.to_string()),
                0,
                Some("work".into()),
            )
            .await
            .unwrap();

        let phrase = instance.recovery_phrase(&second).await.unwrap();

        assert_eq!(phrase.as_str(), SECOND_FIXTURE);
    }

    #[tokio::test]
    async fn standard_address_derives_from_the_profiles_phrase_and_index() {
        let instance = instance();
        instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();
        let (_, second) = instance
            .import_profile(
                Zeroizing::new(SECOND_FIXTURE.to_string()),
                1,
                Some("work".into()),
            )
            .await
            .unwrap();

        let address = instance.standard_address(&second, 2).await.unwrap();

        // `m/44'/60'/1'/0/2` of `SECOND_FIXTURE`.
        assert_eq!(
            address,
            alloy_primitives::address!("0x8D518dd6ebD188c590345C9409F00e47069F3E13")
        );
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
