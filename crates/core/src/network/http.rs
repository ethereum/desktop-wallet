use reqwest::Url;

use super::endpoint::NetworkEndpointError;

/// An http or https URL, trimmed. Any other scheme is rejected.
pub(crate) fn parse_http_url(url: &str) -> Result<Url, NetworkEndpointError> {
    let url = url.trim();
    url.parse::<Url>()
        .ok()
        .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
        .ok_or_else(|| NetworkEndpointError::InvalidUrl(url.to_string()))
}

/// The HTTP client every outbound wallet request should use.
///
/// Clearnet today. A later Tor mode sets a SOCKS proxy on this builder, and both the
/// JSON-RPC endpoint and the Saga CDN pick it up from here.
pub fn outbound_http() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
}
