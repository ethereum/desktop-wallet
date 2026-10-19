/// The HTTP client every outbound wallet request should use.
///
/// Clearnet today. A later Tor mode sets a SOCKS proxy on this builder, and both the
/// JSON-RPC endpoint and the Saga CDN pick it up from here.
pub fn outbound_http() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
}
