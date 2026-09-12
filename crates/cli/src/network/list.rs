use crate::GlobalArgs;

pub async fn run(global: &GlobalArgs) -> Result<(), anyhow::Error> {
    let network = global.gather().await?.preferences().await?;
    println!("{} (chain {})", network.name, network.network_id.0);
    Ok(())
}
