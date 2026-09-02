use std::sync::Arc;

use alloy_provider::{Provider, ProviderBuilder};
use alloy_transport::mock::Asserter;

/// A provider that answers from `asserter`'s FIFO queue instead of a chain, so a build can be
/// driven to completion, or shown to fail before it makes any RPC call at all, without anvil.
pub(crate) fn mocked_provider(asserter: &Asserter) -> Arc<dyn Provider> {
    Arc::new(ProviderBuilder::new().connect_mocked_client(asserter.clone()))
}
