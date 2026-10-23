//! CTAP2 over HID for security keys where there is no WebAuthn API (macOS,
//! Linux; D50, run 8 decision D143), through `ctap-hid-fido2` (MIT; it
//! builds `hidapi`, which needs libudev on Linux). Behind the `fido2-hid`
//! feature, off by default until the owner checked it with a key (see
//! D143); it compiles on every system so that `--all-features` checks it.
//!
//! - The data to sign is the CTAP2 "challenge": the library hashes it with
//!   SHA-256 into the client data hash, which OpenSSH's security keys sign.
//! - The one FIDO device plugged in is asked; it blinks until it is
//!   touched. With none, or several (which one holds the key is not
//!   known), the key is not offered and the next method runs; so it is
//!   for a key that requires user verification (a PIN), which the app
//!   cannot ask for here yet ([`Authenticator::ready`]).
//! - A new key (Create on the security key): Ed25519 when the device can,
//!   else ECDSA P-256, a non-discoverable credential.
//!
//! Nothing here logs key handles or data.

use ctap_hid_fido2::fidokey::GetAssertionArgsBuilder;
use ctap_hid_fido2::fidokey::make_credential::{
    CredentialSupportedKeyType, MakeCredentialArgsBuilder,
};
use ctap_hid_fido2::{FidoKeyHidFactory, LibCfg};
use shellpp_ssh::sk::{
    Assertion, Attested, Authenticator, CreateRequest, SignRequest, SkError,
    USER_VERIFICATION_REQUIRED,
};

/// The first FIDO2 device on USB HID.
#[derive(Clone, Copy, Debug)]
pub struct Hid;

/// Exactly one FIDO device is plugged in.
fn one_device() -> bool {
    ctap_hid_fido2::get_fidokey_devices().len() == 1
}

impl Authenticator for Hid {
    fn ready(&self, flags: u8) -> bool {
        flags & USER_VERIFICATION_REQUIRED == 0 && one_device()
    }

    fn create(&self, request: &CreateRequest) -> Result<Attested, SkError> {
        if request.user_verification {
            return Err(SkError::Failed);
        }
        if !one_device() {
            return Err(SkError::NoAuthenticator);
        }
        let device =
            FidoKeyHidFactory::create(&LibCfg::init()).map_err(|_| SkError::NoAuthenticator)?;
        // Not checked by anyone (no attestation is asked for), but fresh.
        let challenge: [u8; 32] =
            shellpp_core::crypto::random_array().map_err(|_| SkError::Failed)?;
        let args = MakeCredentialArgsBuilder::new(&request.application, &challenge)
            .key_type(CredentialSupportedKeyType::Ed25519)
            .key_type(CredentialSupportedKeyType::Ecdsa256)
            .without_pin_and_uv()
            .build();
        let attestation = device.make_credential_with_args(&args).map_err(|_| {
            tracing::info!("security key made no credential over HID");
            SkError::Failed
        })?;
        Ok(Attested {
            key_handle: attestation.credential_descriptor.id,
            auth_data: attestation.auth_data,
        })
    }

    fn sign(&self, request: &SignRequest) -> Result<Assertion, SkError> {
        if request.flags & USER_VERIFICATION_REQUIRED != 0 {
            tracing::info!("security key needs a PIN: not supported over HID yet");
            return Err(SkError::Failed);
        }
        if !one_device() {
            return Err(SkError::NoAuthenticator);
        }
        let device =
            FidoKeyHidFactory::create(&LibCfg::init()).map_err(|_| SkError::NoAuthenticator)?;
        let args = GetAssertionArgsBuilder::new(&request.application, &request.data)
            .credential_id(&request.key_handle)
            .without_pin_and_uv()
            .build();
        let assertion = device
            .get_assertion_with_args(&args)
            .map_err(|_| {
                tracing::info!("security key did not sign over HID");
                SkError::Failed
            })?
            .into_iter()
            .next()
            .ok_or(SkError::UnknownCredential)?;
        Assertion::from_authenticator_data(&assertion.auth_data, assertion.signature)
            .ok_or(SkError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use shellpp_ssh::sk::SkAlgorithm;

    use super::*;

    /// A key that needs a PIN is not offered, and refused before any
    /// device is asked.
    #[test]
    fn a_pin_key_is_refused_without_a_device() {
        assert!(!Hid.ready(USER_VERIFICATION_REQUIRED | 1));
        let request = SignRequest {
            algorithm: SkAlgorithm::EcdsaP256,
            application: "ssh:".to_owned(),
            key_handle: vec![1; 32],
            flags: USER_VERIFICATION_REQUIRED | 1,
            data: b"data".to_vec(),
        };
        assert_eq!(Hid.sign(&request).unwrap_err(), SkError::Failed);
    }
}
