use std::{num::NonZeroU64, sync::Arc};

use reqwest::Url;

use super::{Instance, InstanceError};
use crate::{
    database::scoped::{ScopedDatabase, ScopedDatabaseExt},
    network::{
        DEFAULT_EVENT_BLOCK_RANGE, DEFAULT_LOCAL_NODE_PORT, LocalNodeConfig, NetworkConfig,
        NetworkConfigKind, NetworkEndpoint, NetworkPreset, SimpleNetworkEndpoint,
        SimpleProviderConfig, db::NetworkDb, verify_network_id,
    },
};

/// Native asset of a network without a preset.
const DEFAULT_NATIVE_ASSET: &str = "eth";

/// A networkConfig to add, with every field the instance can default left open.
pub enum NetworkConfigSpec {
    /// Defaults to the preset's RPC URL.
    SimpleProvider { url: Option<String> },
    /// Defaults to [`DEFAULT_LOCAL_NODE_PORT`].
    LocalNode { port: Option<u16> },
}

impl Instance {
    pub async fn network_configs(&self) -> Result<Vec<NetworkConfig>, InstanceError> {
        Ok(self.preferences().get_network_configs().await?)
    }

    pub async fn active_network_config_name(&self) -> Result<Option<String>, InstanceError> {
        Ok(self.preferences().get_active().await?)
    }

    /// Adds a networkConfig, and makes it active when none is.
    pub async fn add_network_config(
        &self,
        name: String,
        spec: NetworkConfigSpec,
        event_block_range: Option<NonZeroU64>,
    ) -> Result<NetworkConfig, InstanceError> {
        if name.is_empty() {
            return Err(InstanceError::EmptyConfigName);
        }
        let mut configs = self.network_configs().await?;
        if configs.iter().any(|config| config.name == name) {
            return Err(InstanceError::DuplicateConfig(name));
        }

        let preset = NetworkPreset::from_network_id(self.network_id);
        let event_block_range = event_block_range.unwrap_or(DEFAULT_EVENT_BLOCK_RANGE);
        let kind = match spec {
            NetworkConfigSpec::SimpleProvider { url } => {
                let url = match url {
                    Some(url) => {
                        let url = url.trim();
                        if !(url.starts_with("http://") || url.starts_with("https://")) {
                            return Err(InstanceError::InvalidRpcUrl(url.to_string()));
                        }
                        url.to_string()
                    }
                    None => preset
                        .ok_or(InstanceError::MissingRpcUrl(self.network_id))?
                        .rpc_url()
                        .to_string(),
                };
                NetworkConfigKind::SimpleProvider(SimpleProviderConfig {
                    url,
                    event_block_range,
                })
            }
            NetworkConfigSpec::LocalNode { port } => {
                NetworkConfigKind::LocalNode(LocalNodeConfig {
                    port: port.unwrap_or(DEFAULT_LOCAL_NODE_PORT),
                    event_block_range,
                })
            }
        };
        let config = NetworkConfig {
            network_id: self.network_id,
            name,
            native_asset: preset
                .map_or(DEFAULT_NATIVE_ASSET, NetworkPreset::native_asset)
                .to_string(),
            config: kind,
        };

        configs.push(config.clone());
        let preferences = self.preferences();
        preferences.put_network_configs(&configs).await?;
        if preferences.get_active().await?.is_none() {
            preferences.put_active(&config.name).await?;
        }
        Ok(config)
    }

    pub async fn use_network_config(&self, name: &str) -> Result<(), InstanceError> {
        let configs = self.network_configs().await?;
        if !configs.iter().any(|config| config.name == name) {
            return Err(InstanceError::UnknownConfig(name.to_string()));
        }
        self.preferences().put_active(name).await?;
        Ok(())
    }

    /// Connects to `rpc_url`, or else to the active networkConfig, and rejects an endpoint that
    /// serves another network. Costs one round trip.
    pub async fn endpoint(
        &self,
        rpc_url: Option<&str>,
    ) -> Result<Arc<dyn NetworkEndpoint>, InstanceError> {
        let url = match rpc_url {
            Some(url) => url.to_string(),
            None => self.active_network_config().await?.http_rpc_url(),
        };
        let parsed: Url = url
            .parse()
            .map_err(|_| InstanceError::InvalidRpcUrl(url.clone()))?;
        let endpoint: Arc<dyn NetworkEndpoint> = Arc::new(SimpleNetworkEndpoint::new_http(parsed));
        verify_network_id(endpoint.as_ref(), self.network_id)
            .await
            .map_err(|source| InstanceError::UnusableEndpoint {
                url,
                network_id: self.network_id,
                source,
            })?;
        Ok(endpoint)
    }

    pub(super) async fn seed_default_network_config(&self) -> Result<(), InstanceError> {
        let Some(preset) = NetworkPreset::from_network_id(self.network_id) else {
            return Ok(());
        };
        let preferences = self.preferences();
        if preferences.get_network_configs().await?.is_empty() {
            let config = preset.default_config();
            preferences
                .put_network_configs(std::slice::from_ref(&config))
                .await?;
            preferences.put_active(&config.name).await?;
        }
        Ok(())
    }

    /// The active networkConfig, or the first one when none is marked active.
    async fn active_network_config(&self) -> Result<NetworkConfig, InstanceError> {
        let mut configs = self.network_configs().await?;
        if configs.is_empty() {
            return Err(InstanceError::NoConfigs);
        }
        let index = self
            .active_network_config_name()
            .await?
            .and_then(|active| configs.iter().position(|config| config.name == active))
            .unwrap_or(0);
        Ok(configs.swap_remove(index))
    }

    fn preferences(&self) -> ScopedDatabase {
        self.store.clone().scoped(b"preferences")
    }
}
