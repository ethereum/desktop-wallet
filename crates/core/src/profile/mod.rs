use alloy::primitives::{Address, Bytes, U256};

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Call {
    pub target: Address,
    pub data: Bytes,
    pub value: U256,
}

impl Call {
    pub fn new(target: Address, data: Bytes, value: U256) -> Self {
        Self {
            target,
            data,
            value,
        }
    }
}
