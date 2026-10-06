use std::sync::Arc;

use super::{Instance, InstanceError, NETWORK_SCOPE};
use crate::{
    database::scoped::ScopedDatabase,
    network::{DEFAULT_EVENT_BLOCK_RANGE, NetworkEndpoint, NetworkEndpointConfig, db::NetworkDb},
};

impl Instance {
    pub async fn endpoint_configs(&self) -> Result<Vec<NetworkEndpointConfig>, InstanceError> {
        Ok(self.network_db().get_endpoint_configs().await?)
    }

    pub async fn active_endpoint_name(&self) -> Result<Option<String>, InstanceError> {
        Ok(self.network_db().get_active_endpoint().await?)
    }

    /// Adds an endpoint config, and makes it active when none is.
    pub async fn add_endpoint_config(
        &self,
        config: NetworkEndpointConfig,
    ) -> Result<(), InstanceError> {
        if config.name.is_empty() {
            return Err(InstanceError::EmptyEndpointName);
        }
        let mut configs = self.endpoint_configs().await?;
        if configs.iter().any(|existing| existing.name == config.name) {
            return Err(InstanceError::DuplicateEndpoint(config.name));
        }

        let records = self.network_db();
        if records.get_active_endpoint().await?.is_none() {
            records.put_active_endpoint(&config.name).await?;
        }
        configs.push(config);
        records.put_endpoint_configs(&configs).await?;
        Ok(())
    }

    pub async fn use_endpoint_config(&self, name: &str) -> Result<(), InstanceError> {
        let configs = self.endpoint_configs().await?;
        if !configs.iter().any(|config| config.name == name) {
            return Err(InstanceError::UnknownEndpoint(name.to_string()));
        }
        self.network_db().put_active_endpoint(name).await?;
        Ok(())
    }

    /// Connects to `rpc_url`, or else through the active endpoint config, and rejects an
    /// endpoint that serves another network. Costs one round trip.
    pub async fn endpoint(
        &self,
        rpc_url: Option<&str>,
    ) -> Result<Arc<dyn NetworkEndpoint>, InstanceError> {
        let config = match rpc_url {
            Some(url) => {
                NetworkEndpointConfig::http(url.to_string(), url, DEFAULT_EVENT_BLOCK_RANGE)?
            }
            None => self.active_endpoint_config().await?,
        };
        let endpoint = config.connect()?;
        endpoint
            .verify_network_id(self.network.id)
            .await
            .map_err(|source| InstanceError::UnusableEndpoint {
                name: config.name,
                network_id: self.network.id,
                source,
            })?;
        Ok(endpoint)
    }

    /// The active endpoint config, or the first one when none is marked active.
    async fn active_endpoint_config(&self) -> Result<NetworkEndpointConfig, InstanceError> {
        let mut configs = self.endpoint_configs().await?;
        if configs.is_empty() {
            return Err(InstanceError::NoEndpoints);
        }
        let index = self
            .active_endpoint_name()
            .await?
            .and_then(|active| configs.iter().position(|config| config.name == active))
            .unwrap_or(0);
        Ok(configs.swap_remove(index))
    }

    fn network_db(&self) -> ScopedDatabase {
        ScopedDatabase::new(self.store.clone(), NETWORK_SCOPE)
    }
}
