use alloy_primitives::Address;
use zeroize::Zeroizing;

use super::{Instance, InstanceError, NETWORK_SCOPE};
use crate::{
    asset::{self, Erc20Asset},
    database::scoped::ScopedDatabase,
    keyring::Keyring,
    mnemonic::{Mnemonic, MnemonicRecord, scan::EoaScan},
    network::NetworkEndpoint,
    profile::{ProfileDb, ProfileError, ProfileIndex, ProfileRecord},
};

impl Instance {
    /// Opens Tornado with the profile's saved pool policy. Commands need no pool options.
    pub async fn open_tornado_vault(
        &self,
        profile: &ProfileRecord,
        key: alloy_signer::k256::ecdsa::SigningKey,
        endpoint: std::sync::Arc<dyn NetworkEndpoint>,
    ) -> Result<
        crate::vault::tornadocash::TornadoVault<alloy_signer_local::PrivateKeySigner>,
        InstanceError,
    > {
        use crate::vault::tornadocash::{PoolSelection, TornadoVault};
        let vault = TornadoVault::open(key, self.tornado_database(profile), endpoint).await?;
        if self.profile_use_extended_pools(profile).await? {
            Ok(vault)
        } else {
            Ok(vault.with_pool_selection(PoolSelection::Classic))
        }
    }

    /// The profile's Tornado pool policy. Profiles include extended pools unless explicitly opted out.
    pub async fn profile_use_extended_pools(
        &self,
        profile: &ProfileRecord,
    ) -> Result<bool, InstanceError> {
        self.profile_db(profile)
            .use_extended_pools()
            .await
            .map_err(ProfileError::from)
            .map_err(Into::into)
    }

    /// Configures the profile independently of its serialized identity record.
    pub async fn set_profile_use_extended_pools(
        &self,
        profile: &ProfileRecord,
        enabled: bool,
    ) -> Result<(), InstanceError> {
        self.profile_db(profile)
            .put_use_extended_pools(enabled)
            .await
            .map_err(ProfileError::from)
            .map_err(Into::into)
    }

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
    async fn pool_preference_is_profile_scoped_persistent_and_preserves_identity_encoding() {
        let instance = instance();
        let (_, first) = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, Some("first".into()))
            .await
            .unwrap();
        let second = instance
            .add_profile(first.mnemonic_index, Some(1), Some("second".into()))
            .await
            .unwrap();
        let before = postcard::to_stdvec(&instance.profiles().await.unwrap()).unwrap();
        assert!(instance.profile_use_extended_pools(&first).await.unwrap());
        instance
            .set_profile_use_extended_pools(&first, false)
            .await
            .unwrap();
        assert!(!instance.profile_use_extended_pools(&first).await.unwrap());
        assert!(instance.profile_use_extended_pools(&second).await.unwrap());
        assert_eq!(
            before,
            postcard::to_stdvec(&instance.profiles().await.unwrap()).unwrap()
        );
        let reopened = Instance {
            network: instance.network.clone(),
            store: instance.store.clone(),
        };
        let renamed = reopened
            .set_profile_name(
                first.mnemonic_index,
                first.profile_index,
                Some("renamed".into()),
            )
            .await
            .unwrap();
        assert!(!reopened.profile_use_extended_pools(&renamed).await.unwrap());
        assert!(reopened.profile_use_extended_pools(&second).await.unwrap());
    }

    #[tokio::test]
    async fn opening_a_profile_applies_saved_policy_to_deposit_planning() {
        use alloy_primitives::{U64, U256};
        use alloy_transport::mock::Asserter;

        use crate::test_support::mocked_provider;
        let instance = Instance::in_memory(NetworkId(1));
        let (_, profile) = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();
        let amount = U256::from(3 * 10_u128.pow(16));
        // The first open has no saved preference and must use all pools by default.
        for preference in [None, Some(false), Some(true)] {
            if let Some(enabled) = preference {
                instance
                    .set_profile_use_extended_pools(&profile, enabled)
                    .await
                    .unwrap();
            }
            let mock = Asserter::new();
            mock.push_success(&U64::from(1)); // Open vault.
            mock.push_success(&U64::from(1)); // Plan deposits.
            let key = instance
                .mnemonic(&profile)
                .await
                .unwrap()
                .keychain_key(profile.profile_index)
                .unwrap();
            let vault = instance
                .open_tornado_vault(&profile, key, mocked_provider(&mock))
                .await
                .unwrap();
            let plan = vault.deposit_pools(amount).await;
            assert_eq!(plan.is_ok(), preference.unwrap_or(true));
        }
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
