use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use data_dir::DataDir;
pub use networks::NetworkConfigSpec;

use crate::{
    database::{
        Database,
        encrypted::{EncryptedDatabase, EncryptedDatabaseError},
        file::{FileDatabase, FileDatabaseError},
    },
    mnemonic::{MnemonicError, db::MnemonicDatabaseError},
    network::{NetworkId, db::NetworkDatabaseError, endpoint::NetworkEndpointError},
    profile::simple::{ProfileBootstrapError, db::SimpleProfileDatabaseError},
};

mod data_dir;
mod networks;
mod profiles;

/// One opened wallet instance: the encrypted store of a single network.
pub struct Instance {
    network_id: NetworkId,
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
    #[error("networkConfig name cannot be empty")]
    EmptyConfigName,
    #[error("networkConfig `{0}` already exists")]
    DuplicateConfig(String),
    #[error("no networkConfig `{0}`")]
    UnknownConfig(String),
    #[error("no networkConfigs")]
    NoConfigs,
    #[error("network {0} has no default RPC URL; pass one")]
    MissingRpcUrl(NetworkId),
    #[error("RPC URL `{0}` must be an http or https URL")]
    InvalidRpcUrl(String),
    #[error("endpoint {url} cannot be used for {network_id}")]
    UnusableEndpoint {
        url: String,
        network_id: NetworkId,
        #[source]
        source: NetworkEndpointError,
    },
    #[error(transparent)]
    Mnemonic(#[from] MnemonicError),
    #[error(transparent)]
    MnemonicDatabase(#[from] MnemonicDatabaseError),
    #[error(transparent)]
    NetworkDatabase(#[from] NetworkDatabaseError),
    #[error(transparent)]
    Profile(#[from] ProfileBootstrapError),
    #[error(transparent)]
    ProfileDatabase(#[from] SimpleProfileDatabaseError),
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
        Ok(Self {
            network_id,
            store: Self::store(&dir, password, true).await?,
        })
    }

    /// Opens the instance for `network_id`, and creates an empty one on first use.
    ///
    /// An instance without networkConfigs gets its preset's default, if it has a preset.
    pub async fn open_or_create(
        data_dir: &DataDir,
        network_id: NetworkId,
        password: &[u8],
    ) -> Result<Self, InstanceError> {
        let exists = data_dir.has_instance(network_id);
        let instance = Self {
            network_id,
            store: Self::store(&data_dir.instance_dir(network_id), password, exists).await?,
        };
        instance.seed_default_network_config().await?;
        Ok(instance)
    }

    #[must_use]
    pub const fn network_id(&self) -> NetworkId {
        self.network_id
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
