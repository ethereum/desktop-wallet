use super::NetworkId;

const PIMLICO_RPC: &str = "https://public.pimlico.io/v2";

/// The bundler URLs a new wallet on `network_id` starts with, in fallback order.
///
/// Mainnet and Sepolia start on the public Pimlico endpoint. Any other network starts with
/// none, until one is added.
#[must_use]
pub fn default_bundler_urls(network_id: NetworkId) -> Vec<String> {
    match network_id.0 {
        1 | 11_155_111 => vec![format!("{PIMLICO_RPC}/{}/rpc", network_id.0)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_and_sepolia_start_on_pimlico_and_local_starts_empty() {
        assert_eq!(
            default_bundler_urls(NetworkId(1)),
            vec!["https://public.pimlico.io/v2/1/rpc".to_string()]
        );
        assert_eq!(
            default_bundler_urls(NetworkId(11_155_111)),
            vec!["https://public.pimlico.io/v2/11155111/rpc".to_string()]
        );
        assert!(default_bundler_urls(NetworkId(31_337)).is_empty());
    }
}
