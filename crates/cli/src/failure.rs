use std::{error::Error, fmt};

use alloy_transport::TransportError;
use edw_core::{
    asset::AssetError,
    broadcast::BroadcastError,
    bundler::BundlerListError,
    database::encrypted::EncryptedDatabaseError,
    instance::InstanceError,
    mnemonic::MnemonicError,
    network::NetworkEndpointError,
    profile::ProfileError,
    vault::tornadocash::{AmountError, UserOpError},
};
use kohaku_userop_kit::bundler::BundlerError;
use serde::Serialize;

use crate::report::Report;

/// What kind of failure ended a command, which sets its exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Other,
    Usage,
    Locked,
    NotFound,
    Network,
}

/// A failure the CLI raises itself, carrying its class.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct Failure {
    class: Class,
    message: String,
}

/// The `edw/error` document a failed `--non-interactive` command prints.
#[derive(Serialize)]
pub struct ErrorReport {
    class: Class,
    message: String,
}

impl Class {
    /// The class of the outermost cause in `error`'s chain that has one, else [`Class::Other`].
    fn of(error: &anyhow::Error) -> Self {
        error.chain().find_map(classify).unwrap_or(Self::Other)
    }

    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Other => 1,
            Self::Usage => 2,
            Self::Locked => 3,
            Self::NotFound => 4,
            Self::Network => 5,
        }
    }
}

impl ErrorReport {
    pub fn new(error: &anyhow::Error) -> Self {
        Self {
            class: Class::of(error),
            message: format!("{error:#}"),
        }
    }

    pub const fn class(&self) -> Class {
        self.class
    }
}

impl Report for ErrorReport {
    const KIND: &'static str = "edw/error";
    const VERSION: u32 = 1;
}

impl fmt::Display for ErrorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub fn usage(message: impl Into<String>) -> anyhow::Error {
    failure(Class::Usage, message)
}

pub fn locked(message: impl Into<String>) -> anyhow::Error {
    failure(Class::Locked, message)
}

pub fn not_found(message: impl Into<String>) -> anyhow::Error {
    failure(Class::NotFound, message)
}

fn failure(class: Class, message: impl Into<String>) -> anyhow::Error {
    Failure {
        class,
        message: message.into(),
    }
    .into()
}

// `Error::source` skips the error inside a `#[error(transparent)]` variant, so those variants
// are unwrapped here by hand. `None` leaves the class to the error's source, if any.
fn classify(cause: &(dyn Error + 'static)) -> Option<Class> {
    if let Some(failure) = cause.downcast_ref::<Failure>() {
        Some(failure.class)
    } else if let Some(error) = cause.downcast_ref::<InstanceError>() {
        instance(error)
    } else if let Some(error) = cause.downcast_ref::<NetworkEndpointError>() {
        Some(endpoint(error))
    } else if let Some(error) = cause.downcast_ref::<ProfileError>() {
        profile(error)
    } else if let Some(error) = cause.downcast_ref::<MnemonicError>() {
        mnemonic(error)
    } else if let Some(error) = cause.downcast_ref::<AmountError>() {
        amount(error)
    } else if let Some(error) = cause.downcast_ref::<UserOpError>() {
        user_op(error)
    } else if let Some(error) = cause.downcast_ref::<BroadcastError>() {
        broadcast(error)
    } else if let Some(error) = cause.downcast_ref::<BundlerListError>() {
        bundler_list(error)
    } else if let Some(error) = cause.downcast_ref::<EncryptedDatabaseError>() {
        encrypted_database(error)
    } else {
        cause.is::<BundlerError>().then_some(Class::Network)
    }
}

fn instance(error: &InstanceError) -> Option<Class> {
    match error {
        InstanceError::NotFound { .. }
        | InstanceError::UnknownRpcConfig(_)
        | InstanceError::NoRpcConfigs
        | InstanceError::UnknownBundlerUrl(_) => Some(Class::NotFound),
        InstanceError::EmptyRpcConfigName
        | InstanceError::DuplicateRpcConfig(_)
        | InstanceError::DuplicateBundlerUrl(_)
        | InstanceError::LastBundlerUrl
        | InstanceError::IndexerUrlRequired => Some(Class::Usage),
        InstanceError::Endpoint(error) => Some(endpoint(error)),
        InstanceError::Mnemonic(error) => mnemonic(error),
        InstanceError::Profile(error) => profile(error),
        InstanceError::Asset(error) => asset(error),
        InstanceError::AddressVaultDatabase(_)
        | InstanceError::Open { .. }
        | InstanceError::Create(_)
        | InstanceError::Unlock(_)
        | InstanceError::MissingNetwork(_)
        | InstanceError::UnusableRpcConfig { .. }
        | InstanceError::NetworkDatabase(_)
        | InstanceError::VaultDatabase(_) => None,
    }
}

fn endpoint(error: &NetworkEndpointError) -> Class {
    match error {
        NetworkEndpointError::InvalidUrl(_) => Class::Usage,
        NetworkEndpointError::Backend(error) if answered(error.as_ref()) => Class::Other,
        NetworkEndpointError::Backend(_) | NetworkEndpointError::NetworkMismatch { .. } => {
            Class::Network
        }
    }
}

/// Whether the node answered with an error, such as a revert, that retrying would not change.
fn answered(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<TransportError>()
        .and_then(TransportError::as_error_resp)
        .is_some_and(|response| !response.is_retry_err())
}

const fn asset(error: &AssetError) -> Option<Class> {
    match error {
        AssetError::NotTracked(_) => Some(Class::NotFound),
        AssetError::NoBalance(_) | AssetError::NoDecimals(_) => Some(Class::Usage),
        AssetError::Database(_) | AssetError::Network(_) | AssetError::Unregistered(_) => None,
    }
}

const fn profile(error: &ProfileError) -> Option<Class> {
    match error {
        ProfileError::Unresolved(_) => Some(Class::NotFound),
        ProfileError::Ambiguous(_)
        | ProfileError::Duplicate { .. }
        | ProfileError::DuplicateName(_)
        | ProfileError::IndexOutOfRange(_) => Some(Class::Usage),
        ProfileError::Database(_) => None,
    }
}

const fn mnemonic(error: &MnemonicError) -> Option<Class> {
    match error {
        MnemonicError::WordCount
        | MnemonicError::Phrase(_)
        | MnemonicError::DuplicatePhrase { .. } => Some(Class::Usage),
        MnemonicError::Unresolved(_) => Some(Class::NotFound),
        MnemonicError::Path(_)
        | MnemonicError::Derivation(_)
        | MnemonicError::SigningKey
        | MnemonicError::TooMany
        | MnemonicError::ScanLimit(_)
        | MnemonicError::Database(_)
        | MnemonicError::Network(_) => None,
    }
}

const fn amount(error: &AmountError) -> Option<Class> {
    match error {
        AmountError::Zero | AmountError::NotADenominationSum { .. } | AmountError::Parse(_) => {
            Some(Class::Usage)
        }
        AmountError::NoNativePools { .. }
        | AmountError::CannotMake { .. }
        | AmountError::CannotCover { .. } => None,
    }
}

const fn user_op(error: &UserOpError) -> Option<Class> {
    match error {
        UserOpError::Bundler(_) => Some(Class::Network),
        UserOpError::Empty
        | UserOpError::MissingPaymaster(_)
        | UserOpError::Short { .. }
        | UserOpError::Unstable { .. }
        | UserOpError::FeeMissing
        | UserOpError::FeeDecode(_)
        | UserOpError::Reverted { .. }
        | UserOpError::Vault(_)
        | UserOpError::Paymaster(_)
        | UserOpError::SmartAccount(_)
        | UserOpError::Sign(_) => None,
    }
}

const fn broadcast(error: &BroadcastError) -> Option<Class> {
    match error {
        BroadcastError::Bundler(_) => Some(Class::Network),
        BroadcastError::BundlerList(error) => bundler_list(error),
        BroadcastError::Network(_)
        | BroadcastError::Signer(_)
        | BroadcastError::Delegate(_)
        | BroadcastError::Builder(_)
        | BroadcastError::NotMined { .. }
        | BroadcastError::Reverted { .. }
        | BroadcastError::EmptyUserOperation
        | BroadcastError::NoProvider
        | BroadcastError::Account(_)
        | BroadcastError::Signature(_)
        | BroadcastError::UserOperationReceipt { .. }
        | BroadcastError::UserOperationReverted { .. } => None,
    }
}

const fn bundler_list(error: &BundlerListError) -> Option<Class> {
    match error {
        BundlerListError::Empty | BundlerListError::Url { .. } => Some(Class::Usage),
        BundlerListError::Client(_) => None,
    }
}

const fn encrypted_database(error: &EncryptedDatabaseError) -> Option<Class> {
    match error {
        EncryptedDatabaseError::InvalidPassword | EncryptedDatabaseError::EmptyPassword => {
            Some(Class::Locked)
        }
        EncryptedDatabaseError::Database(_)
        | EncryptedDatabaseError::AlreadyInitialized
        | EncryptedDatabaseError::NotInitialized
        | EncryptedDatabaseError::Corrupt
        | EncryptedDatabaseError::UnsupportedVersion(_)
        | EncryptedDatabaseError::UnsupportedParameters
        | EncryptedDatabaseError::KeyDerivation
        | EncryptedDatabaseError::NoMatchingSlot(_)
        | EncryptedDatabaseError::Serialization(_) => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use alloy_json_rpc::ErrorPayload;
    use alloy_primitives::Address;
    use anyhow::Context;
    use kohaku_tornadocash::userop_provider::TornadoPaymasterError;

    use super::*;
    use crate::report::Mode;

    #[test]
    fn context_keeps_the_class_of_the_cause() {
        let error = Err::<(), _>(locked("wallet is locked"))
            .context("opening the wallet")
            .unwrap_err();
        assert_eq!(Class::of(&error), Class::Locked);
    }

    #[test]
    fn a_transparent_endpoint_error_is_network() {
        let error = anyhow::Error::from(InstanceError::Endpoint(NetworkEndpointError::Backend(
            "connection refused".into(),
        )));
        assert_eq!(Class::of(&error), Class::Network);
    }

    #[test]
    fn a_node_error_response_is_other() {
        let response: TransportError = TransportError::ErrorResp(
            ErrorPayload::internal_error_message("execution reverted".into()),
        );
        let error = anyhow::Error::from(NetworkEndpointError::Backend(Box::new(response)));
        assert_eq!(Class::of(&error), Class::Other);
    }

    #[test]
    fn a_rate_limited_response_is_network() {
        let response: TransportError = TransportError::ErrorResp(ErrorPayload {
            code: 429,
            message: "too many requests".into(),
            data: None,
        });
        let error = anyhow::Error::from(NetworkEndpointError::Backend(Box::new(response)));
        assert_eq!(Class::of(&error), Class::Network);
    }

    #[test]
    fn a_transparent_asset_error_is_classed() {
        let error =
            anyhow::Error::from(InstanceError::Asset(AssetError::NotTracked(Address::ZERO)));
        assert_eq!(Class::of(&error), Class::NotFound);
    }

    #[test]
    fn a_transparent_mnemonic_error_is_usage() {
        let error = anyhow::Error::from(InstanceError::Mnemonic(MnemonicError::WordCount));
        assert_eq!(Class::of(&error), Class::Usage);
    }

    #[test]
    fn a_bundler_failure_under_the_paymaster_is_network() {
        let error = anyhow::Error::from(UserOpError::from(TornadoPaymasterError::Bundler(
            BundlerError::Timeout,
        )));
        assert_eq!(Class::of(&error), Class::Network);
    }

    #[test]
    fn a_wrong_password_under_unlock_is_locked() {
        let error = anyhow::Error::from(InstanceError::Unlock(
            EncryptedDatabaseError::InvalidPassword,
        ));
        assert_eq!(Class::of(&error), Class::Locked);
    }

    #[test]
    fn an_unclassified_error_documents_as_other_with_its_full_message() {
        let error = anyhow::anyhow!("no profiles").context("choosing a profile");
        assert_eq!(
            ErrorReport::new(&error).format(Mode::Json).unwrap(),
            r#"{"kind":"edw/error","version":1,"class":"other","message":"choosing a profile: no profiles"}"#
        );
    }
}
