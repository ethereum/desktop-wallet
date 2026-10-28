//! Where a person asks to send funds: an address, or an ENS name.
//!
//! Names resolve through the ENS Universal Resolver over the network endpoint. A name whose
//! resolver redirects to an off-chain CCIP-Read gateway is refused rather than followed, so
//! resolution never contacts a server other than the endpoint.

use std::{
    fmt,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

use alloy_ens::{EnsError, ProviderEnsExt, UNIVERSAL_RESOLVER_ADDRESS};
use alloy_primitives::{Address, Bytes};
use alloy_provider::{CcipReadClient, CcipReadGateway, CcipReadGatewayError, CcipReadRequest};

use crate::network::NetworkEndpoint;

/// An address, or an ENS name still to be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipient {
    Address(Address),
    /// An ASCII name, already in its ENSIP-15 normalized form.
    Name(String),
}

/// A recipient and, when one was given, the name it was resolved from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRecipient {
    pub name: Option<String>,
    pub address: Address,
}

#[derive(Default)]
struct NoGateway {
    asked: AtomicBool,
}

#[derive(Debug, thiserror::Error)]
pub enum RecipientError {
    #[error("`{0}` fails its EIP-55 checksum, so it may be mistyped")]
    Checksum(String),
    #[error("`{0}` is neither an address nor an ENS name")]
    Invalid(String),
    #[error("`{0}` is the zero address, and anything sent to it is lost")]
    Zero(String),
    #[error(
        "`{0}` is not a supported ENS name; only ASCII names that ENSIP-15 accepts resolve, others need the address"
    )]
    UnsupportedName(String),
    #[error("this network has no ENS Universal Resolver, so names cannot be resolved")]
    NoEns,
    #[error("{0} has no ENS address record")]
    NoRecord(String),
    #[error(
        "{0} resolves through an off-chain CCIP-Read gateway, which the wallet does not contact; pass the address"
    )]
    Offchain(String),
    #[error("this endpoint cannot resolve ENS names")]
    NoProvider,
    #[error("checking for ENS: {0}")]
    Network(#[from] crate::network::NetworkEndpointError),
    #[error("resolving {name}: {source}")]
    Ens { name: String, source: Box<EnsError> },
}

#[async_trait::async_trait]
impl CcipReadGateway for NoGateway {
    async fn request(
        &self,
        _request: &CcipReadRequest,
        _max_response_size: usize,
    ) -> Result<Bytes, CcipReadGatewayError> {
        self.asked.store(true, Ordering::Relaxed);
        Err(CcipReadGatewayError::new(
            "off-chain gateways are not contacted",
        ))
    }
}

impl FromStr for Recipient {
    type Err = RecipientError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if Address::from_str(text).is_ok() {
            return recipient_address(text).map(Self::Address);
        }
        if !text.contains('.') {
            return Err(RecipientError::Invalid(text.to_string()));
        }
        normalized_name(text).map(Self::Name)
    }
}

impl fmt::Display for Recipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(address) => address.fmt(f),
            Self::Name(name) => name.fmt(f),
        }
    }
}

impl fmt::Display for ResolvedRecipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name} ({})", self.address),
            None => self.address.fmt(f),
        }
    }
}

impl Recipient {
    /// The address to send to.
    ///
    /// # Errors
    /// [`RecipientError::NoRecord`] when the name has no address, [`RecipientError::Offchain`]
    /// when it needs a CCIP-Read gateway, [`RecipientError::NoEns`] on a network without the
    /// Universal Resolver, and [`RecipientError::Network`] or [`RecipientError::Ens`] when the
    /// endpoint or resolution fails.
    pub async fn resolve(
        self,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<ResolvedRecipient, RecipientError> {
        let name = match self {
            Self::Address(address) => {
                return Ok(ResolvedRecipient {
                    name: None,
                    address,
                });
            }
            Self::Name(name) => name,
        };
        let provider = endpoint.provider().ok_or(RecipientError::NoProvider)?;
        if endpoint
            .code_at(UNIVERSAL_RESOLVER_ADDRESS)
            .await?
            .is_empty()
        {
            return Err(RecipientError::NoEns);
        }
        let client = CcipReadClient::new(NoGateway::default());
        let address = provider
            .resolve_name_with_ccip_read(&name, &client)
            .await
            .map_err(|source| match source {
                // A refused lookup inside a local batch comes back as the resolver's revert.
                _ if client.gateway().asked.load(Ordering::Relaxed) => {
                    RecipientError::Offchain(name.clone())
                }
                EnsError::ResolverNotFound(_) => RecipientError::NoRecord(name.clone()),
                source => RecipientError::Ens {
                    name: name.clone(),
                    source: Box::new(source),
                },
            })?;
        if address.is_zero() {
            return Err(RecipientError::NoRecord(name));
        }
        Ok(ResolvedRecipient {
            name: Some(name),
            address,
        })
    }
}

/// Parses an address, rejecting a mixed-case one whose EIP-55 checksum does not match.
///
/// All-lowercase and all-uppercase addresses carry no checksum and are accepted.
///
/// # Errors
/// [`RecipientError::Invalid`] when the text is not an address, and
/// [`RecipientError::Checksum`] when its checksum does not match.
pub fn parse_address(text: &str) -> Result<Address, RecipientError> {
    let address = Address::from_str(text).map_err(|_| RecipientError::Invalid(text.to_string()))?;
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    let mixed_case = digits.bytes().any(|byte| byte.is_ascii_uppercase())
        && digits.bytes().any(|byte| byte.is_ascii_lowercase());
    if mixed_case && Address::parse_checksummed(format!("0x{digits}"), None).is_err() {
        return Err(RecipientError::Checksum(text.to_string()));
    }
    Ok(address)
}

/// Parses an address to send to, as [`parse_address`] does, and refuses the zero address.
///
/// # Errors
/// As [`parse_address`], and [`RecipientError::Zero`] for the zero address.
pub fn recipient_address(text: &str) -> Result<Address, RecipientError> {
    let address = parse_address(text)?;
    if address.is_zero() {
        return Err(RecipientError::Zero(text.to_string()));
    }
    Ok(address)
}

/// ENSIP-15 normalization, limited to ASCII labels.
fn normalized_name(text: &str) -> Result<String, RecipientError> {
    let name = text.to_ascii_lowercase();
    let label_ok = |label: &str| {
        let body = label.trim_start_matches('_');
        !label.is_empty()
            && body
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'$')
            && label.get(2..4) != Some("--")
    };
    if name.split('.').all(label_ok) {
        Ok(name)
    } else {
        Err(RecipientError::UnsupportedName(text.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKSUMMED: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    fn refused(text: &str) -> bool {
        matches!(
            text.parse::<Recipient>(),
            Err(RecipientError::UnsupportedName(_))
        )
    }

    #[test]
    fn a_checksummed_or_single_case_address_is_accepted() {
        for text in [
            CHECKSUMMED,
            "0xd8da6bf26964af9d7eed9e03e53415d37aa96045",
            "0xD8DA6BF26964AF9D7EED9E03E53415D37AA96045",
            "0Xd8da6bf26964af9d7eed9e03e53415d37aa96045",
        ] {
            assert!(parse_address(text).is_ok(), "{text} must parse");
        }
    }

    #[test]
    fn a_mixed_case_address_with_a_wrong_checksum_is_refused() {
        let wrong = CHECKSUMMED.replacen("dA", "Da", 1);
        assert!(matches!(
            parse_address(&wrong),
            Err(RecipientError::Checksum(_))
        ));
    }

    #[test]
    fn the_zero_address_is_refused_as_a_recipient() {
        assert!(matches!(
            "0x0000000000000000000000000000000000000000".parse::<Recipient>(),
            Err(RecipientError::Zero(_))
        ));
    }

    #[test]
    fn a_name_is_lowercased() {
        assert_eq!(
            "Vitalik.ETH".parse::<Recipient>().ok(),
            Some(Recipient::Name("vitalik.eth".to_string()))
        );
    }

    #[test]
    fn a_name_outside_ascii_is_refused() {
        assert!(refused("caf\u{e9}.eth"));
    }

    #[test]
    fn an_empty_label_is_refused() {
        assert!(refused("a..eth"));
        assert!(refused("vitalik.eth."));
    }

    #[test]
    fn an_underscore_is_allowed_only_at_the_start_of_a_label() {
        assert!(!refused("__a.eth"));
        assert!(refused("a_b.eth"));
    }

    #[test]
    fn a_dollar_sign_is_allowed() {
        assert_eq!(
            "$a.eth".parse::<Recipient>().ok(),
            Some(Recipient::Name("$a.eth".to_string()))
        );
    }

    #[test]
    fn hyphens_as_the_third_and_fourth_character_are_refused() {
        assert!(!refused("a-b-c.eth"));
        assert!(refused("ab--c.eth"));
    }

    #[tokio::test]
    async fn a_name_on_a_network_without_ens_is_refused_before_resolving() {
        let asserter = alloy_transport::mock::Asserter::new();
        asserter.push_success(&Bytes::new());
        let resolved = Recipient::Name("vitalik.eth".to_string())
            .resolve(crate::test_support::mocked_provider(&asserter).as_ref())
            .await;
        assert!(matches!(resolved, Err(RecipientError::NoEns)));
    }

    #[test]
    fn text_that_is_neither_is_invalid() {
        assert!(matches!(
            "0x1234".parse::<Recipient>(),
            Err(RecipientError::Invalid(_))
        ));
    }
}
