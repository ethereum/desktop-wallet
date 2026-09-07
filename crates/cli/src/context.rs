use std::sync::Arc;

use edw_core::database::Database;

use crate::GlobalArgs;

pub struct Context {
    pub networks: Arc<dyn Database>,
}

impl GlobalArgs {
    pub async fn gather(&self) -> anyhow::Result<Context> {
        super::unlock::run_unlock(self).await?;
        let networks = super::unlock::network_store(&self.data_dir).await?;

        Ok(Context { networks })
    }
}
