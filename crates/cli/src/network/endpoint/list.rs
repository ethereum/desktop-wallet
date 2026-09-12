use clap::Args;
use edw_core::network::{Network, endpoint::NetworkEndpointConfig};

#[derive(Args, Debug)]
pub struct NetworkEndpointListArgs {
    /// Prints endpoint URLs.
    #[arg(long)]
    show_urls: bool,
}

impl NetworkEndpointListArgs {
    pub fn run(&self, network: &Network) {
        if network.endpoints.is_empty() {
            println!("No endpoints configured.");
            println!("Set one with `edw network set-rpc <url>`.");
            return;
        }

        for endpoint in &network.endpoints {
            match endpoint {
                NetworkEndpointConfig::HttpProvider { url } => {
                    if self.show_urls {
                        println!("http {url}");
                    } else {
                        println!("http (hidden; pass --show-urls)");
                    }
                }
            }
        }
    }
}
