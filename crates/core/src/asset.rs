use alloy_primitives::Address;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetId {
    Native,
    Erc20(Address),
}

impl AssetId {
    #[must_use]
    pub fn native() -> Self {
        AssetId::Native
    }

    #[must_use]
    pub fn erc20(address: Address) -> Self {
        AssetId::Erc20(address)
    }
}
