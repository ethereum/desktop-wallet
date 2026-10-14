use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::TransactionRequest;
use edw_core::call::Call;

/// Sends `calls` one at a time from the provider's wallet, waiting for each to be mined.
pub async fn send_calls(
    provider: &DynProvider,
    calls: &[Call],
) -> Result<(), Box<dyn std::error::Error>> {
    for call in calls {
        let tx = TransactionRequest::default()
            .to(call.target)
            .input(call.data.clone().into())
            .value(call.value);
        provider.send_transaction(tx).await?.watch().await?;
    }
    Ok(())
}
