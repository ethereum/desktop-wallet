//! Simulation funding compensates only the SDK's temporary 10% sponsor fee.
//! Real withdrawals still execute. No override is used to validate or submit the final operation.
use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::state::{StateOverride, StateOverridesBuilder};
use kohaku_tornadocash::Pool;
use kohaku_userop_kit::{
    bundler::{Bundler, BundlerError},
    signable_user_operation::SignableUserOperation,
    signed_user_operation::SignedUserOperation,
    user_operation::{UserOperationGasEstimate, UserOperationHash, UserOperationReceipt},
};

use super::{Asset, UserOpError};

pub(super) struct FundingBundler<'a> {
    pub inner: &'a (dyn Bundler + Send + Sync),
    pub funding: &'a StateOverride,
}

#[async_trait::async_trait]
impl Bundler for FundingBundler<'_> {
    async fn estimate_gas(
        &self,
        op: &SignableUserOperation,
        state: Option<&StateOverride>,
    ) -> Result<UserOperationGasEstimate, BundlerError> {
        let mut overrides = state.cloned().unwrap_or_default();
        // The funding and verifier overrides concern different accounts. Merge fields to avoid
        // discarding delegation code or other SDK overrides if that ever changes.
        for (address, funding) in self.funding {
            let entry = overrides.entry(*address).or_default();
            if let Some(balance) = funding.balance {
                entry.balance = Some(balance);
            }
            if let Some(diff) = &funding.state_diff {
                entry.state_diff.get_or_insert_default().extend(diff);
            }
        }
        let mut gas = self.inner.estimate_gas(op, Some(&overrides)).await?;
        // Account for storage transitions and calldata changes between placeholder and real proof.
        gas.call_gas_limit = pad(gas.call_gas_limit);
        gas.verification_gas_limit = pad(gas.verification_gas_limit);
        gas.pre_verification_gas = pad(gas.pre_verification_gas);
        gas.paymaster_verification_gas_limit = gas.paymaster_verification_gas_limit.map(pad);
        gas.paymaster_post_op_gas_limit = gas.paymaster_post_op_gas_limit.map(pad);
        Ok(gas)
    }
    async fn send_user_operation(
        &self,
        op: &SignedUserOperation,
    ) -> Result<UserOperationHash, BundlerError> {
        self.inner.send_user_operation(op).await
    }
    async fn receipt(
        &self,
        hash: UserOperationHash,
    ) -> Result<Option<UserOperationReceipt>, BundlerError> {
        self.inner.receipt(hash).await
    }
    async fn wait_for_receipt(
        &self,
        hash: UserOperationHash,
    ) -> Result<UserOperationReceipt, BundlerError> {
        self.inner.wait_for_receipt(hash).await
    }
}
pub(super) async fn seed_overrides(
    provider: &DynProvider,
    pool: &Pool,
    owner: Address,
) -> Result<StateOverride, UserOpError> {
    if pool.asset != Asset::ETH {
        return Err(UserOpError::Invalid(
            "only ETH notes may sponsor withdrawals".into(),
        ));
    }
    let seed = U256::from(pool.amount_wei) / U256::from(10);
    let balance = provider
        .get_balance(owner)
        .await
        .map_err(|e| UserOpError::Simulation(e.to_string()))?;
    Ok(StateOverridesBuilder::default()
        .with_balance(owner, balance.saturating_add(seed))
        .build())
}

fn pad(gas: u128) -> u128 {
    gas.saturating_add(gas / 5).saturating_add(1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use alloy_node_bindings::Anvil;
    use alloy_provider::ProviderBuilder;

    use super::*;

    #[tokio::test]
    async fn native_seed_funding_is_only_a_simulation_override() {
        let anvil = Anvil::new().spawn();
        let provider = ProviderBuilder::new()
            .connect_http(anvil.endpoint_url())
            .erased();
        let owner = Address::repeat_byte(0x42);
        let pool = Pool::ETHEREUM_ETHER_01;
        let overrides = seed_overrides(&provider, &pool, owner).await.unwrap();
        assert_eq!(
            overrides[&owner].balance,
            Some(U256::from(pool.amount_wei / 10))
        );
        assert_eq!(provider.get_balance(owner).await.unwrap(), U256::ZERO);
    }
}
