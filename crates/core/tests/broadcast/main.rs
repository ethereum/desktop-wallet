use std::sync::Arc;

use alloy_node_bindings::{Anvil, AnvilInstance};
use alloy_primitives::{Address, Bytes, U256, bytes};
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_signer::k256::ecdsa::SigningKey;
use edw_core::{
    broadcast::{prepare_call, submit_prepared},
    call::Call,
    network::SimpleNetworkEndpoint,
    signer::simple::SimpleSigner,
};

const RECIPIENT: Address = Address::repeat_byte(0x42);
const PRICED_ONLY: Address = Address::repeat_byte(0x43);

fn connect(
    anvil: &AnvilInstance,
) -> Result<(DynProvider, SimpleSigner), Box<dyn std::error::Error>> {
    let provider = ProviderBuilder::new()
        .connect_http(anvil.endpoint().parse()?)
        .erased();
    let key = SigningKey::from(anvil.first_key().clone());
    Ok((provider, SimpleSigner::from_key(key)))
}

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn the_reported_fee_is_what_the_sender_paid_beyond_the_value()
-> Result<(), Box<dyn std::error::Error>> {
    let anvil = Anvil::new().prague().spawn();
    let (provider, signer) = connect(&anvil)?;
    let endpoint = SimpleNetworkEndpoint::new(provider.clone());
    let sender = signer.address();
    let value = U256::from(1_000_000_u64);
    let before = provider.get_balance(sender).await?;

    let prepared = prepare_call(
        &endpoint,
        sender,
        &Call::new(RECIPIENT, Bytes::new(), value),
    )
    .await?;
    let included = submit_prepared(&endpoint, Arc::new(signer), prepared).await?;

    let after = provider.get_balance(sender).await?;
    assert!(!included.fee.is_zero());
    assert_eq!(before - after - value, included.fee);
    Ok(())
}

#[tokio::test]
#[ignore = "run with `cargo test -- --ignored`"]
async fn sending_to_a_contract_that_checks_the_gas_price_succeeds()
-> Result<(), Box<dyn std::error::Error>> {
    let anvil = Anvil::new().prague().spawn();
    let (provider, signer) = connect(&anvil)?;
    // GASPRICE ISZERO PUSH1 6 JUMPI STOP JUMPDEST PUSH0 PUSH0 REVERT
    let code = bytes!("3a15600657005b5f5ffd");
    let _: () = provider
        .raw_request("anvil_setCode".into(), (PRICED_ONLY, code))
        .await?;
    let endpoint = SimpleNetworkEndpoint::new(provider);
    let sender = signer.address();

    let prepared = prepare_call(
        &endpoint,
        sender,
        &Call::new(PRICED_ONLY, Bytes::new(), U256::from(1_u8)),
    )
    .await?;
    submit_prepared(&endpoint, Arc::new(signer), prepared).await?;
    Ok(())
}
