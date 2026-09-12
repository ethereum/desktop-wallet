use std::sync::Arc;

use edw_core::{
    database::{
        Database,
        scoped::{ScopedDatabase, ScopedDatabaseExt},
    },
    network::{Network, SupportedNetwork, db::NetworkDb},
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

    pub fn profile_db(&self, name: &str) -> ScopedDatabase {
        self.store
            .clone()
            .scoped(format!("profile:{name}").as_bytes())
    }

    pub async fn preferences(&self) -> anyhow::Result<Network> {
        self.preferences_db()
            .get_network()
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing network preferences; run `edw unlock`"))
    }
}

impl GlobalArgs {
    pub async fn gather(&self) -> anyhow::Result<Context> {
        let session = session::load().ok_or_else(unlock::locked_error)?;
        let data_dir = session::canonical_data_dir(&self.data_dir);
        if session.data_dir != data_dir {
            anyhow::bail!(
                "wallet is unlocked for {} at {}, not {}; run `edw unlock --network <mainnet|sepolia|local>`",
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
