use std::sync::Arc;

use edw_core::{
    database::{
        Database,
        scoped::{ScopedDatabase, ScopedDatabaseExt},
    },
    mnemonic::{MnemonicRecord, db::MnemonicDb},
    network::{
        NetworkConfig, NetworkEndpoint, NetworkId, SimpleNetworkEndpoint, SupportedNetwork,
        db::NetworkDb,
    },
    profile::simple::{ProfileRecord, db::SimpleProfileDb, profile_scope},
};

use crate::{GlobalArgs, session, unlock};

pub struct Context {
    pub network: SupportedNetwork,
    pub store: Arc<dyn Database>,
}

impl Context {
    pub fn preferences_db(&self) -> ScopedDatabase {
        self.store.clone().scoped(b"preferences")
    }

    pub fn profiles_index_db(&self) -> ScopedDatabase {
        self.store.clone().scoped(b"profiles")
    }

    pub fn mnemonics_db(&self) -> ScopedDatabase {
        self.store.clone().scoped(b"mnemonics")
    }

    pub fn profile_db(&self, mnemonic_index: u32, profile_index: u32) -> ScopedDatabase {
        self.store
            .clone()
            .scoped(profile_scope(mnemonic_index, profile_index).as_bytes())
    }

    pub async fn mnemonics(&self) -> anyhow::Result<Vec<MnemonicRecord>> {
        Ok(self.mnemonics_db().get_mnemonics().await?)
    }

    pub async fn profiles(&self) -> anyhow::Result<Vec<ProfileRecord>> {
        Ok(self.profiles_index_db().list_profiles().await?)
    }

    pub fn chain_id(&self) -> NetworkId {
        self.network.default_config().network_id
    }

    pub async fn network_configs(&self) -> anyhow::Result<Vec<NetworkConfig>> {
        Ok(self.preferences_db().get_network_configs().await?)
    }

    pub async fn put_network_configs(&self, configs: &[NetworkConfig]) -> anyhow::Result<()> {
        let chain = self.chain_id();
        for config in configs {
            if config.network_id != chain {
                anyhow::bail!(
                    "networkConfig `{}` must be chain {}, not {}",
                    config.name,
                    chain.0,
                    config.network_id.0
                );
            }
        }
        self.preferences_db().put_network_configs(configs).await?;
        Ok(())
    }

    pub async fn resolve_config(
        &self,
        name: Option<&str>,
    ) -> anyhow::Result<(usize, Vec<NetworkConfig>)> {
        let configs = self.network_configs().await?;
        if configs.is_empty() {
            anyhow::bail!("no networkConfigs; add one with `edw network add <name> <type>`");
        }
        if let Some(name) = name {
            let index = configs
                .iter()
                .position(|config| config.name == name)
                .ok_or_else(|| anyhow::anyhow!("no networkConfig `{name}`"))?;
            return Ok((index, configs));
        }
        if let Some(active) = self.preferences_db().get_active().await?
            && let Some(index) = configs.iter().position(|config| config.name == active)
        {
            return Ok((index, configs));
        }
        Ok((0, configs))
    }

    pub async fn endpoint(
        &self,
        rpc_url: Option<&str>,
    ) -> anyhow::Result<Arc<dyn NetworkEndpoint>> {
        let url = if let Some(url) = rpc_url {
            url.to_string()
        } else {
            let (index, configs) = self.resolve_config(None).await?;
            configs[index].http_rpc_url()
        };
        Ok(Arc::new(SimpleNetworkEndpoint::new_http(url.parse()?)))
    }
}

impl GlobalArgs {
    pub async fn gather(&self) -> anyhow::Result<Context> {
        let session = session::load().ok_or_else(unlock::locked_error)?;
        let data_dir = session::canonical_data_dir(&self.data_dir);
        if session.data_dir != data_dir {
            anyhow::bail!(
                "wallet is unlocked for {} at {}, not {}; run `edw unlock [--network mainnet|sepolia|local]`",
                session.network,
                session.data_dir.display(),
                data_dir.display()
            );
        }

        Ok(Context {
            network: session.network,
            store: unlock::open_existing_store(
                &session.data_dir,
                session.network,
                session.password.as_bytes(),
            )
            .await?,
        })
    }
}
