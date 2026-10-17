use std::sync::Arc;

use alloy_primitives::Address;
use futures::future::join_all;

use super::{Instance, InstanceError};
use crate::{
    database::{Database, scoped::ScopedDatabase},
    factory::{BuildContext, Factory, FactoryError},
    mnemonic::scan::EoaScan,
    network::NetworkEndpoint,
    profile::ProfileRecord,
    vault::{
        Vault,
        address::{
            ADDRESS_VAULT_TAG,
            db::{AddressVaultDb, DerivedAddress},
        },
        db::{VaultDb, VaultRecord},
    },
};

impl Instance {
    /// The vaults `profile` holds, in the order they were added.
    pub async fn vault_records(
        &self,
        profile: &ProfileRecord,
    ) -> Result<Vec<VaultRecord>, InstanceError> {
        Ok(self.profile_db(profile).vault_records().await?)
    }

    /// A record and an empty database for a new vault of `tag` on `profile`.
    ///
    /// The vault is not listed until [`Self::add_vault`] is called with the record, so one whose
    /// setup fails before then is never built.
    #[must_use]
    pub fn new_vault(
        &self,
        profile: &ProfileRecord,
        tag: &str,
    ) -> (VaultRecord, Arc<dyn Database>) {
        let record = VaultRecord::new(tag);
        let db = self.vault_db(profile, &record);
        (record, db)
    }

    /// Lists `record` among `profile`'s vaults.
    pub async fn add_vault(
        &self,
        profile: &ProfileRecord,
        record: VaultRecord,
    ) -> Result<(), InstanceError> {
        let db = self.profile_db(profile);
        let mut records = db.vault_records().await?;
        records.push(record);
        db.put_vault_records(&records).await?;
        Ok(())
    }

    /// Adds an address vault to `profile` for each address in `scan`, or for its address 0 when
    /// `scan` found none or is `None`.
    ///
    /// Does not check for vaults `profile` already has for the same addresses.
    pub async fn add_scanned_address_vaults(
        &self,
        profile: &ProfileRecord,
        scan: Option<&EoaScan>,
    ) -> Result<(), InstanceError> {
        let addresses = match scan {
            Some(scan) if !scan.addresses.is_empty() => scan.addresses.clone(),
            _ => vec![(0, self.standard_address(profile, 0).await?)],
        };
        self.add_address_vaults(profile, &addresses).await
    }

    /// Builds each of `records` against `endpoint`, concurrently and in order. A vault that fails
    /// to build keeps its error, and does not stop the others.
    ///
    /// `records` must be `profile`'s: each vault is built from its database in `profile`'s scope.
    pub async fn build_vaults(
        &self,
        profile: &ProfileRecord,
        records: &[VaultRecord],
        endpoint: &Arc<dyn NetworkEndpoint>,
    ) -> Vec<Result<Box<dyn Vault>, FactoryError>> {
        join_all(records.iter().map(|record| {
            Factory::<dyn Vault>::build(
                &record.tag,
                BuildContext::new(endpoint.clone(), self.vault_db(profile, record)),
            )
        }))
        .await
    }

    async fn add_address_vaults(
        &self,
        profile: &ProfileRecord,
        addresses: &[(u32, Address)],
    ) -> Result<(), InstanceError> {
        for &(index, address) in addresses {
            let (record, db) = self.new_vault(profile, ADDRESS_VAULT_TAG);
            db.put_derived_address(&DerivedAddress { address, index })
                .await?;
            self.add_vault(profile, record).await?;
        }
        Ok(())
    }

    fn vault_db(&self, profile: &ProfileRecord, record: &VaultRecord) -> Arc<dyn Database> {
        Arc::new(ScopedDatabase::new(
            Arc::new(self.profile_db(profile)),
            record.scope().as_bytes(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_transport::mock::Asserter;
    use zeroize::Zeroizing;

    use super::*;
    use crate::{
        asset::AssetId,
        network::NetworkId,
        test_support::mocked_provider,
        vault::{Balance, VaultError, VaultId, Visibility},
    };

    const STORED_TAG: &str = "test-stored";
    const FIXTURE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    /// `m/44'/60'/0'/0/0` of `FIXTURE`.
    const FIXTURE_ADDRESS_0: Address = address!("0x9858EfFD232B4033E47d90003D41EC34EcaEda94");

    /// Holds the amount its factory read from the vault's database.
    struct StoredVault(U256);

    inventory::submit! {
        Factory::<dyn Vault>::new(STORED_TAG, |ctx: BuildContext| {
            Box::pin(async move {
                let bytes = ctx
                    .db
                    .get(b"amount")
                    .await
                    .map_err(|e| FactoryError::Other(Box::new(e)))?
                    .unwrap_or_default();
                Ok(Box::new(StoredVault(U256::from_be_slice(&bytes))) as Box<dyn Vault>)
            })
        })
    }

    #[async_trait::async_trait]
    impl Vault for StoredVault {
        fn tag(&self) -> &'static str {
            STORED_TAG
        }

        fn id(&self) -> VaultId {
            VaultId::Address(Address::ZERO)
        }

        fn visibility(&self) -> Visibility {
            Visibility::Public
        }

        async fn balance(&self, _asset: &AssetId) -> Result<Balance, VaultError> {
            Ok(Balance::spendable(self.0))
        }
    }

    fn instance() -> Instance {
        Instance::in_memory(NetworkId(1337))
    }

    fn profile(profile_index: u32) -> ProfileRecord {
        ProfileRecord::new(0, profile_index, None)
    }

    async fn imported(instance: &Instance) -> ProfileRecord {
        let (_, profile) = instance
            .import_profile(Zeroizing::new(FIXTURE.to_string()), 0, None)
            .await
            .unwrap();
        profile
    }

    async fn derived_addresses(
        instance: &Instance,
        profile: &ProfileRecord,
    ) -> Vec<(u32, Address)> {
        let mut derived = Vec::new();
        for record in instance.vault_records(profile).await.unwrap() {
            let address = instance
                .vault_db(profile, &record)
                .get_derived_address()
                .await
                .unwrap();
            derived.push((address.index, address.address));
        }
        derived
    }

    async fn new_stored(instance: &Instance, profile: &ProfileRecord, amount: u8) -> VaultRecord {
        let (record, db) = instance.new_vault(profile, STORED_TAG);
        db.put(b"amount", &[amount]).await.unwrap();
        record
    }

    #[tokio::test]
    async fn added_vaults_are_listed_in_order() {
        let instance = instance();
        let (first, second) = (VaultRecord::new("a"), VaultRecord::new("b"));
        instance
            .add_vault(&profile(0), first.clone())
            .await
            .unwrap();
        instance
            .add_vault(&profile(0), second.clone())
            .await
            .unwrap();
        assert_eq!(
            instance.vault_records(&profile(0)).await.unwrap(),
            [first, second]
        );
    }

    #[tokio::test]
    async fn a_new_vault_is_not_listed_until_added() {
        let instance = instance();
        let _ = instance.new_vault(&profile(0), STORED_TAG);
        assert!(
            instance
                .vault_records(&profile(0))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_vault_is_listed_only_for_its_profile() {
        let instance = instance();
        instance
            .add_vault(&profile(0), VaultRecord::new("a"))
            .await
            .unwrap();
        assert!(
            instance
                .vault_records(&profile(1))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn each_vault_is_built_from_its_own_database() {
        let instance = instance();
        let records = [
            new_stored(&instance, &profile(0), 1).await,
            new_stored(&instance, &profile(0), 2).await,
        ];
        let endpoint = mocked_provider(&Asserter::new());
        let mut balances = Vec::new();
        for vault in instance
            .build_vaults(&profile(0), &records, &endpoint)
            .await
        {
            balances.push(
                vault
                    .unwrap()
                    .balance(&AssetId::Native)
                    .await
                    .unwrap()
                    .spendable,
            );
        }
        assert_eq!(balances, [U256::from(1), U256::from(2)]);
    }

    #[tokio::test]
    async fn an_unknown_tag_fails_only_its_own_vault() {
        let instance = instance();
        let records = [
            VaultRecord::new("no-such-vault"),
            new_stored(&instance, &profile(0), 1).await,
        ];
        let endpoint = mocked_provider(&Asserter::new());
        let built = instance
            .build_vaults(&profile(0), &records, &endpoint)
            .await;
        assert!(built[0].is_err());
        assert!(built[1].is_ok());
    }

    #[tokio::test]
    async fn an_address_vault_builds_for_its_address() {
        let instance = instance();
        let address = Address::repeat_byte(0xaa);
        instance
            .add_address_vaults(&profile(0), &[(3, address)])
            .await
            .unwrap();

        let records = instance.vault_records(&profile(0)).await.unwrap();
        let endpoint = mocked_provider(&Asserter::new());
        let built = instance
            .build_vaults(&profile(0), &records, &endpoint)
            .await;
        assert_eq!(built[0].as_ref().unwrap().id(), VaultId::Address(address));
    }

    #[tokio::test]
    async fn each_address_in_a_scan_becomes_a_vault() {
        let instance = instance();
        let profile = imported(&instance).await;
        let used = vec![
            (3, Address::repeat_byte(0xaa)),
            (15, Address::repeat_byte(0xbb)),
        ];
        let scan = EoaScan {
            addresses: used.clone(),
            last_used: Some(15),
            next_unused: 16,
        };

        instance
            .add_scanned_address_vaults(&profile, Some(&scan))
            .await
            .unwrap();

        assert_eq!(derived_addresses(&instance, &profile).await, used);
    }

    #[tokio::test]
    async fn without_a_used_address_address_0_becomes_a_vault() {
        let no_used = EoaScan {
            addresses: Vec::new(),
            last_used: None,
            next_unused: 0,
        };
        for scan in [Some(&no_used), None] {
            let instance = instance();
            let profile = imported(&instance).await;

            instance
                .add_scanned_address_vaults(&profile, scan)
                .await
                .unwrap();

            assert_eq!(
                derived_addresses(&instance, &profile).await,
                [(0, FIXTURE_ADDRESS_0)]
            );
        }
    }
}
