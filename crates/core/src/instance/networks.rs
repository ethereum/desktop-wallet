use std::sync::Arc;

use super::{Instance, InstanceError, NETWORK_SCOPE};
use crate::{
    database::scoped::ScopedDatabase,
    network::{NetworkEndpoint, RpcConfig, db::NetworkDb},
};

impl Instance {
    pub async fn rpc_configs(&self) -> Result<Vec<RpcConfig>, InstanceError> {
        Ok(self.network_db().get_rpc_configs().await?)
    }

    pub async fn active_rpc_config_name(&self) -> Result<Option<String>, InstanceError> {
        Ok(self.network_db().get_active_rpc_config().await?)
    }

    /// Adds an RPC config, and makes it active when none is.
    pub async fn add_rpc_config(&self, config: RpcConfig) -> Result<(), InstanceError> {
        if config.name.is_empty() {
            return Err(InstanceError::EmptyRpcConfigName);
        }
        let mut configs = self.rpc_configs().await?;
        if configs.iter().any(|existing| existing.name == config.name) {
            return Err(InstanceError::DuplicateRpcConfig(config.name));
        }

        let records = self.network_db();
        if records.get_active_rpc_config().await?.is_none() {
            records.put_active_rpc_config(&config.name).await?;
        }
        configs.push(config);
        records.put_rpc_configs(&configs).await?;
        Ok(())
    }

    pub async fn use_rpc_config(&self, name: &str) -> Result<(), InstanceError> {
        let configs = self.rpc_configs().await?;
        if !configs.iter().any(|config| config.name == name) {
            return Err(InstanceError::UnknownRpcConfig(name.to_string()));
        }
        self.network_db().put_active_rpc_config(name).await?;
        Ok(())
    }

    /// Connects through the active RPC config, and rejects one that serves another network.
    /// Costs one round trip.
    pub async fn rpc(&self) -> Result<Arc<dyn NetworkEndpoint>, InstanceError> {
        let config = self.active_rpc_config().await?;
        let endpoint = config.connect()?;
        endpoint
            .verify_network_id(self.network.id)
            .await
            .map_err(|source| InstanceError::UnusableRpcConfig {
                name: config.name,
                network_id: self.network.id,
                source,
            })?;
        Ok(endpoint)
    }

    /// The public RPC for this network, recorded and made active. No-op when this network has none.
    pub(super) async fn install_default_rpc_config(&self) -> Result<(), InstanceError> {
        let Some(config) = RpcConfig::public_default(self.network.id) else {
            return Ok(());
        };
        self.add_rpc_config(config).await
    }

    /// The active RPC config, or the first one when none is marked active.
    async fn active_rpc_config(&self) -> Result<RpcConfig, InstanceError> {
        let mut configs = self.rpc_configs().await?;
        if configs.is_empty() {
            return Err(InstanceError::NoRpcConfigs);
        }
        let index = self
            .active_rpc_config_name()
            .await?
            .and_then(|active| configs.iter().position(|config| config.name == active))
            .unwrap_or(0);
        Ok(configs.swap_remove(index))
    }

    fn network_db(&self) -> ScopedDatabase {
        ScopedDatabase::new(self.store.clone(), NETWORK_SCOPE)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::network::NetworkId;

    #[tokio::test]
    async fn a_mainnet_instance_starts_with_publicnode_when_asked() {
        let instance = super::super::Instance::in_memory(NetworkId(1));
        instance.install_default_rpc_config().await.unwrap();
        assert_eq!(
            instance.active_rpc_config_name().await.unwrap().as_deref(),
            Some("publicnode")
        );
    }

    #[tokio::test]
    async fn a_local_instance_starts_with_no_rpc_config() {
        let instance = super::super::Instance::in_memory(NetworkId(31_337));
        instance.install_default_rpc_config().await.unwrap();
        assert!(instance.rpc_configs().await.unwrap().is_empty());
    }
}
