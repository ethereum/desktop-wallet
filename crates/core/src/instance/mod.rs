use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use assets::AccountBalances;
pub use data_dir::DataDir;

use crate::{
    asset::{AssetError, AssetId},
    database::{
        Database,
        encrypted::{EncryptedDatabase, EncryptedDatabaseError},
        file::{FileDatabase, FileDatabaseError},
        scoped::ScopedDatabase,
    },
    mnemonic::MnemonicError,
    network::{
        Network, NetworkId,
        db::{NetworkDatabaseError, NetworkDb},
        endpoint::NetworkEndpointError,
    },
    profile::ProfileError,
};

mod assets;
mod data_dir;
mod networks;
mod profiles;

/// Scope holding the [`Network`] record and its endpoint configs.
const NETWORK_SCOPE: &[u8] = b"network";

/// One opened wallet instance: the encrypted store of a single network.
pub struct Instance {
    network: Network,
    store: Arc<dyn Database>,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("no wallet instance for {network_id} at {}", dir.display())]
    NotFound { network_id: NetworkId, dir: PathBuf },
    #[error("error opening the store at {}", dir.display())]
    Open {
        dir: PathBuf,
        #[source]
        source: FileDatabaseError,
    },
    #[error("error creating the store")]
    Create(#[source] EncryptedDatabaseError),
    #[error("error unlocking the store")]
    Unlock(#[source] EncryptedDatabaseError),
    #[error("the {0} instance has no network record; run `edw unlock --network {0}`")]
    MissingNetwork(NetworkId),
    #[error("endpoint name cannot be empty")]
    EmptyEndpointName,
    #[error("endpoint `{0}` already exists")]
    DuplicateEndpoint(String),
    #[error("no endpoint `{0}`")]
    UnknownEndpoint(String),
    #[error(
        "no endpoint configured; add your own with `edw network endpoint add <name> --url <url>`"
    )]
    NoEndpoints,
    #[error("endpoint `{name}` cannot be used for {network_id}")]
    UnusableEndpoint {
        name: String,
        network_id: NetworkId,
        #[source]
        source: NetworkEndpointError,
    },
    #[error("asset {0} is already configured")]
    DuplicateAsset(AssetId),
    #[error("no asset `{0}`")]
    UnknownAsset(String),
    #[error("`{0}` names more than one asset; use its symbol")]
    AmbiguousAsset(String),
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error(transparent)]
    Endpoint(#[from] NetworkEndpointError),
    #[error(transparent)]
    Mnemonic(#[from] MnemonicError),
    #[error(transparent)]
    NetworkDatabase(#[from] NetworkDatabaseError),
    #[error(transparent)]
    Profile(#[from] ProfileError),
}

impl Instance {
    /// Opens the existing instance for `network_id`. Creates nothing.
    pub async fn open(
        data_dir: &DataDir,
        network_id: NetworkId,
        password: &[u8],
    ) -> Result<Self, InstanceError> {
        let dir = data_dir.instance_dir(network_id);
        if !data_dir.has_instance(network_id) {
            return Err(InstanceError::NotFound { network_id, dir });
        }
        let store = Self::store(&dir, password, true).await?;
        let network = ScopedDatabase::new(store.clone(), NETWORK_SCOPE)
            .get_network()
            .await?
            .ok_or(InstanceError::MissingNetwork(network_id))?;
        Ok(Self { network, store })
    }

    /// Opens the instance for `network_id`, and creates an empty one on first use.
    ///
    /// Records the network on first use. Never adds an endpoint: the user brings their own.
    pub async fn open_or_create(
        data_dir: &DataDir,
        network_id: NetworkId,
        password: &[u8],
    ) -> Result<Self, InstanceError> {
        let exists = data_dir.has_instance(network_id);
        let store = Self::store(&data_dir.instance_dir(network_id), password, exists).await?;
        let records = ScopedDatabase::new(store.clone(), NETWORK_SCOPE);
        let network = if let Some(network) = records.get_network().await? {
            network
        } else {
            let network = Network::new(network_id);
            records.put_network(&network).await?;
            network
        };
        Ok(Self { network, store })
    }

    #[must_use]
    pub const fn network(&self) -> &Network {
        &self.network
    }

    /// An instance over an unencrypted memory store, for tests that exercise its logic.
    #[cfg(test)]
    fn in_memory(network_id: NetworkId) -> Self {
        Self {
            network: Network::new(network_id),
            store: Arc::new(crate::database::memory::MemoryDatabase::new()),
        }
    }

    async fn store(
        dir: &Path,
        password: &[u8],
        exists: bool,
    ) -> Result<Arc<dyn Database>, InstanceError> {
        let backend: Arc<dyn Database> =
            Arc::new(
                FileDatabase::open(dir).map_err(|source| InstanceError::Open {
                    dir: dir.to_path_buf(),
                    source,
                })?,
            );
        let store = if exists {
            EncryptedDatabase::unlock(backend, password)
                .await
                .map_err(InstanceError::Unlock)?
        } else {
            EncryptedDatabase::create(backend, password)
                .await
                .map_err(InstanceError::Create)?
        };
        Ok(Arc::new(store))
    }
}
