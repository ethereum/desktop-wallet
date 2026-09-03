//! Shared fixture for the anvil-backed tests.
//!
//! Every anvil-backed test spawns its chain through [`devnet`] rather than calling
//! [`Anvil::new`] directly, so the chain's shape is pinned in one place instead of being
//! whatever the local toolchain happens to default to.

// Each test binary uses a subset of this module.
#![allow(dead_code)]

use alloy_node_bindings::{Anvil, AnvilInstance};

/// Anvil's own default mnemonic, set explicitly so the accounts a test derives stay fixed
/// even if that default changes.
const MNEMONIC: &str = "test test test test test test test test test test test junk";

/// Spawns a local devnet on a pinned hardfork with a fixed mnemonic.
///
/// Anvil otherwise follows the local foundry build's notion of "latest", which differs
/// between a contributor's machine and the version nix pins for CI. EIP-7702 needs Prague.
#[must_use]
pub fn devnet() -> AnvilInstance {
    Anvil::new().prague().mnemonic(MNEMONIC).spawn()
}

/// Installs a tracing subscriber for the current test binary.
///
/// Uses `try_init` rather than `init`, which panics on a second call, so adding a second test
/// to any of these binaries does not turn into a confusing failure.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .try_init();
}
