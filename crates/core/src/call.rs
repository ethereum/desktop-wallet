use alloy::primitives::{Address, Bytes, U256};

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
