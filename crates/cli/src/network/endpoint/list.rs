use clap::Args;
use edw_core::network::Network;

#[derive(Args, Debug)]
pub struct NetworkEndpointListArgs {
    /// Prints endpoint URLs.
    #[arg(long)]
    show_urls: bool,
}

impl NetworkEndpointListArgs {
    pub fn run(&self, network: &Network) {
        println!("Network endpoints for: {}", network.name);

        for x in &network.endpoints {
            println!("config, {x:?}");
            if self.show_urls {
                println!("showing url");
            }
        }
    }
}
