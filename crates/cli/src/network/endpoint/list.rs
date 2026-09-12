use clap::Args;
use edw_core::network::{NetworkConfig, endpoint::NetworkEndpointConfig};

#[derive(Args, Debug)]
pub struct NetworkEndpointListArgs {
    /// Prints endpoint URLs.
    #[arg(long)]
    show_urls: bool,
    /// networkConfig to list. Defaults to the active config.
    #[arg(long)]
    pub name: Option<String>,
}

impl NetworkEndpointListArgs {
    pub fn run(&self, config: &NetworkConfig) {
        if config.endpoints.is_empty() {
            println!("No endpoints configured.");
            println!("Set one with `edw network set-rpc <url>`.");
            return;
        }

        for endpoint in &config.endpoints {
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
