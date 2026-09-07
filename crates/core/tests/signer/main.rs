//! Cargo infers an integration test target from `tests/*.rs` and `tests/*/main.rs` only, so
//! removing this file stops everything in this directory from running, silently.

mod simple;
