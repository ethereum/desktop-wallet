use alloy::primitives::Address;

pub mod executor;
pub mod signer;
pub mod vault;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExecutorId {
    Address(Address),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SignerId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VaultId {
    Address(Address),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetId {
    Native,
    Erc20(Address),
}
