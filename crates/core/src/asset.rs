use alloy_primitives::Address;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetId {
    Native,
    Erc20(Address),
}

impl AssetId {
    pub fn native() -> Self {
        AssetId::Native
    }

    pub fn erc20(address: Address) -> Self {
        AssetId::Erc20(address)
    }
}
