use std::path::{Path, PathBuf};

use alloy_provider::{Provider, ProviderBuilder};
use alloy_transport::mock::Asserter;

use crate::network::alloy::SimpleNetworkEndpoint;

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> Self {
        Self(std::env::temp_dir().join(format!("edw-test-{}", uuid::Uuid::new_v4())))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An endpoint that answers from `asserter`'s FIFO queue instead of a chain.
pub(crate) fn mocked_provider(asserter: &Asserter) -> SimpleNetworkEndpoint {
    SimpleNetworkEndpoint::new(
        ProviderBuilder::new()
            .connect_mocked_client(asserter.clone())
            .erased(),
    )
}
