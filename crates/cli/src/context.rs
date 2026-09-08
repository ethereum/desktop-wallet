use std::sync::Arc;

use edw_core::database::Database;

use crate::GlobalArgs;

pub struct Context {
    pub networks: Arc<dyn Database>,
}

impl GlobalArgs {
    /// Unlocks the wallet and opens the network store, creating it if needed.
    pub async fn network_writer(&self) -> anyhow::Result<Context> {
        Ok(Context {
            networks: super::unlock::network_store(&self.data_dir).await?,
        })
    }
}
