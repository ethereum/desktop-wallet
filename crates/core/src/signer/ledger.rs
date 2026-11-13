use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::eip7702::Authorization;
use alloy_network::TxSigner;
use alloy_primitives::{Address, Signature};
use alloy_signer::Signer as AlloySigner;
use alloy_signer_ledger::{
    HDPath, LedgerError, LedgerSigner,
    coins_ledger::{
        common::APDUResponseCodes, errors::LedgerError as TransportError,
        transports::native::NativeTransportError,
    },
};

use crate::{
    mnemonic::{
        MnemonicError,
        scan::{EoaScan, scan_with},
    },
    network::NetworkEndpoint,
    signer::{Signer, SignerError, SignerId},
};

pub const LEDGER_SIGNER_TAG: &str = "ledger-signer";

/// A [`Signer`] for one standard address on a connected Ledger running the Ethereum app.
///
/// Each signature waits for confirmation on the device. Not registered with the signer
/// factory, so an executor or vault cannot be persisted over it.
pub struct LedgerAccount {
    signer: LedgerSigner,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerAccountError {
    #[error(transparent)]
    Device(LedgerError),
    #[error("the Ledger is not ready; connect it, unlock it, and open its Ethereum app")]
    NotReady,
    #[error("the Ledger did not sign")]
    Signing(alloy_signer::Error),
    #[error("rejected on the Ledger")]
    Rejected,
    #[error("the connected Ledger is not the one this profile was created from")]
    WrongDevice,
    #[error("derivation index {0} is too large; it must be below 2^31")]
    Index(u32),
    #[error("this Ledger signer cannot sign EIP-7702 authorizations")]
    AuthorizationUnsupported,
    #[error(transparent)]
    Scan(#[from] MnemonicError),
}

impl LedgerAccount {
    /// Connects to the first Ledger found, for standard address `address_index` of
    /// `profile_index`. The device must be unlocked with the Ethereum app open.
    pub async fn connect(
        profile_index: u32,
        address_index: u32,
    ) -> Result<Self, LedgerAccountError> {
        let signer = LedgerSigner::new(standard_path(address_index, profile_index)?, None)
            .await
            .map_err(device_error)?;
        Ok(Self { signer })
    }

    /// The device's address at `m/44'/60'/0'/0/0`, which tells one seed from another.
    pub async fn identity(&self) -> Result<Address, LedgerAccountError> {
        self.standard_address(0, 0).await
    }

    /// Reads a standard address from the device, without a prompt on it.
    pub async fn standard_address(
        &self,
        address_index: u32,
        profile_index: u32,
    ) -> Result<Address, LedgerAccountError> {
        self.signer
            .get_address_with_path(&standard_path(address_index, profile_index)?)
            .await
            .map_err(device_error)
    }

    /// [`crate::mnemonic::Mnemonic::scan_standard_eoas`], reading each address from the device.
    pub async fn scan_standard_eoas(
        &self,
        profile_index: u32,
        endpoint: &dyn NetworkEndpoint,
    ) -> Result<EoaScan, LedgerAccountError> {
        scan_with(
            |address_index| self.standard_address(address_index, profile_index),
            |address| async move {
                Ok(endpoint
                    .has_activity(address)
                    .await
                    .map_err(MnemonicError::from)?)
            },
        )
        .await
    }
}

#[async_trait::async_trait]
impl Signer for LedgerAccount {
    fn tag(&self) -> &'static str {
        LEDGER_SIGNER_TAG
    }

    fn id(&self) -> SignerId {
        SignerId::Address(self.address())
    }

    fn address(&self) -> Address {
        AlloySigner::address(&self.signer)
    }

    async fn personal_sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_message(message)
            .await
            .map_err(signing_error)?;
        Ok(signature)
    }

    async fn sign_typed_data(&self, data: &TypedData) -> Result<Signature, SignerError> {
        let signature = self
            .signer
            .sign_dynamic_typed_data(data)
            .await
            .map_err(signing_error)?;
        Ok(signature)
    }

    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature, SignerError> {
        let signature = TxSigner::sign_transaction(&self.signer, tx)
            .await
            .map_err(signing_error)?;
        Ok(signature)
    }

    async fn sign_authorization(
        &self,
        _authorization: &Authorization,
    ) -> Result<Signature, SignerError> {
        Err(LedgerAccountError::AuthorizationUnsupported.into())
    }
}

impl From<LedgerAccountError> for SignerError {
    fn from(err: LedgerAccountError) -> Self {
        match err {
            LedgerAccountError::Rejected => SignerError::Rejected,
            err => SignerError::Other(Box::new(err)),
        }
    }
}

/// The Ethereum app answers a denial on the device with status `0x6985`.
fn signing_error(error: alloy_signer::Error) -> LedgerAccountError {
    let device = match &error {
        alloy_signer::Error::Other(source) => source.downcast_ref::<LedgerError>(),
        _ => None,
    };
    match device {
        Some(LedgerError::LedgerError(TransportError::BadRetcode(
            APDUResponseCodes::ConditionsNotSatisfied,
        ))) => LedgerAccountError::Rejected,
        Some(device) if is_not_ready(device) => LedgerAccountError::NotReady,
        _ => LedgerAccountError::Signing(error),
    }
}

fn device_error(error: LedgerError) -> LedgerAccountError {
    if is_not_ready(&error) {
        LedgerAccountError::NotReady
    } else {
        LedgerAccountError::Device(error)
    }
}

/// A locked Ledger is not found, and with no app open the signer reports the status as an empty
/// response. The transport names `0x6804` a locked device, and `0x6D00` and `0x6E00` reject an
/// instruction or class the open app does not know.
fn is_not_ready(error: &LedgerError) -> bool {
    matches!(
        error,
        LedgerError::UnexpectedNullResponse
            | LedgerError::LedgerError(
                TransportError::NativeTransportError(NativeTransportError::DeviceNotFound)
                    | TransportError::BadRetcode(
                        APDUResponseCodes::UnlockDeviceError
                            | APDUResponseCodes::ClaNotSupported
                            | APDUResponseCodes::InsNotSupported
                    )
            )
    )
}

fn standard_path(address_index: u32, profile_index: u32) -> Result<HDPath, LedgerAccountError> {
    for index in [address_index, profile_index] {
        if index >= 1 << 31 {
            return Err(LedgerAccountError::Index(index));
        }
    }
    Ok(HDPath::Other(format!(
        "m/44'/60'/{profile_index}'/0/{address_index}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_status(code: APDUResponseCodes) -> alloy_signer::Error {
        alloy_signer::Error::other(LedgerError::LedgerError(TransportError::BadRetcode(code)))
    }

    #[test]
    fn a_denial_on_the_device_is_a_rejection() {
        let error = SignerError::from(signing_error(device_status(
            APDUResponseCodes::ConditionsNotSatisfied,
        )));

        assert!(matches!(error, SignerError::Rejected));
    }

    #[test]
    fn a_closed_ethereum_app_is_not_ready() {
        let error = device_error(LedgerError::UnexpectedNullResponse);

        assert!(matches!(error, LedgerAccountError::NotReady));
    }

    #[test]
    fn a_locked_or_missing_device_is_not_ready() {
        let error = device_error(LedgerError::LedgerError(
            TransportError::NativeTransportError(NativeTransportError::DeviceNotFound),
        ));

        assert!(matches!(error, LedgerAccountError::NotReady));
    }

    #[test]
    fn a_device_that_stops_answering_before_it_signs_is_not_ready() {
        let error = signing_error(alloy_signer::Error::other(
            LedgerError::UnexpectedNullResponse,
        ));

        assert!(matches!(error, LedgerAccountError::NotReady));
    }

    #[test]
    fn another_app_open_is_not_ready() {
        let error = device_error(LedgerError::LedgerError(TransportError::BadRetcode(
            APDUResponseCodes::ClaNotSupported,
        )));

        assert!(matches!(error, LedgerAccountError::NotReady));
    }

    #[test]
    fn the_standard_path_puts_the_profile_before_the_address() {
        assert_eq!(standard_path(2, 1).unwrap().to_string(), "m/44'/60'/1'/0/2");
    }

    #[test]
    fn a_hardened_address_index_is_refused() {
        let error = standard_path(1 << 31, 0).unwrap_err();

        assert!(matches!(error, LedgerAccountError::Index(index) if index == 1 << 31));
    }

    #[test]
    fn any_other_device_status_stays_a_signing_error() {
        let error = signing_error(device_status(APDUResponseCodes::InvalidData));

        assert!(matches!(error, LedgerAccountError::Signing(_)));
    }
}
