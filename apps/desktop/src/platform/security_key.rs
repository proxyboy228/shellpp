//! The FIDO2 authenticator of security keys (D50, run 8 decision D143):
//! on Windows the system's WebAuthn API (`webauthn.dll`, Windows 10 1903
//! and later), which shows its own "Windows Security" dialog to touch the
//! key or enter its PIN; elsewhere CTAP2 over HID ([`super::security_key_hid`],
//! the `fido2-hid` feature). Without either a security key is unusable and
//! the next method runs.
//!
//! - The WebAuthn call gets the data to sign as its "client data": the API
//!   hashes it with SHA-256, which is the client data hash OpenSSH's
//!   security keys sign (PROTOCOL.u2f). The relying party id is the key's
//!   application (`ssh:`), the allowed credential its key handle.
//! - A test run never reaches an authenticator (no dialogs in tests).
//!
//! A platform module (CONTRIBUTING.md, "Unsafe code"): the WebAuthn and
//! `GetForegroundWindow` calls are its only `unsafe` code. Nothing here
//! logs key handles or data.
#![cfg_attr(windows, allow(unsafe_code))]

use std::sync::Arc;

use shellpp_ssh::sk::Authenticator;

/// The authenticator for security keys on this system, if there is one.
pub fn authenticator() -> Option<Arc<dyn Authenticator>> {
    if crate::dirs::is_test_run() {
        return None;
    }
    system()
}

/// Whether this system can make a new key on a security key (WebAuthn;
/// the HID path signs only).
pub fn can_create() -> bool {
    cfg!(windows) && authenticator().is_some()
}

#[cfg(windows)]
fn system() -> Option<Arc<dyn Authenticator>> {
    windows::available().then(|| Arc::new(windows::WebAuthn) as Arc<dyn Authenticator>)
}

#[cfg(all(not(windows), feature = "fido2-hid"))]
fn system() -> Option<Arc<dyn Authenticator>> {
    Some(Arc::new(super::security_key_hid::Hid))
}

#[cfg(all(not(windows), not(feature = "fido2-hid")))]
fn system() -> Option<Arc<dyn Authenticator>> {
    None
}

/// What an `HRESULT` of the WebAuthn API means for the sign-in.
#[cfg(any(windows, test))]
fn error_of(hresult: i32) -> shellpp_ssh::sk::SkError {
    use shellpp_ssh::sk::SkError;
    // The bit pattern of an HRESULT: compare as unsigned.
    match hresult as u32 {
        // NTE_USER_CANCELLED; HRESULT_FROM_WIN32(ERROR_CANCELLED);
        // HRESULT_FROM_WIN32(ERROR_TIMEOUT).
        0x8009_0036 | 0x8007_04C7 | 0x8007_05B4 => SkError::Cancelled,
        // NTE_NOT_FOUND: the key does not hold the credential.
        0x8009_0011 => SkError::UnknownCredential,
        // NTE_DEVICE_NOT_FOUND.
        0x8009_0035 => SkError::NoAuthenticator,
        _ => SkError::Failed,
    }
}

#[cfg(windows)]
mod windows {
    use std::ptr;

    use shellpp_ssh::sk::{
        Assertion, Attested, Authenticator, CreateRequest, SignRequest, SkError,
        USER_VERIFICATION_REQUIRED,
    };
    use windows_sys::Win32::Networking::WindowsWebServices::{
        WEBAUTHN_ASSERTION, WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
        WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
        WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_1, WEBAUTHN_CLIENT_DATA,
        WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, WEBAUTHN_CREDENTIAL,
        WEBAUTHN_CREDENTIAL_CURRENT_VERSION, WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
        WEBAUTHN_CREDENTIALS, WEBAUTHN_HASH_ALGORITHM_SHA_256,
        WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
        WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED, WebAuthNAuthenticatorGetAssertion,
        WebAuthNFreeAssertion, WebAuthNGetApiVersionNumber,
    };
    use windows_sys::Win32::Networking::WindowsWebServices::{
        WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS,
        WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_1,
        WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256, WEBAUTHN_COSE_CREDENTIAL_PARAMETER,
        WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION, WEBAUTHN_COSE_CREDENTIAL_PARAMETERS,
        WEBAUTHN_CREDENTIAL_ATTESTATION, WEBAUTHN_RP_ENTITY_INFORMATION,
        WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION, WEBAUTHN_USER_ENTITY_INFORMATION,
        WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION, WebAuthNAuthenticatorMakeCredential,
        WebAuthNFreeCredentialAttestation,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    /// How long the system dialog waits for the touch.
    const TIMEOUT_MS: u32 = 60_000;
    /// COSE EdDSA (RFC 8152); windows-sys names only ES256.
    const COSE_EDDSA: i32 = -8;

    /// A NUL-terminated UTF-16 string.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A copy of `len` bytes at `data` (none for a null pointer).
    ///
    /// # Safety
    /// `data` is null or points to `len` readable bytes.
    unsafe fn copy(data: *const u8, len: u32) -> Vec<u8> {
        if data.is_null() {
            return Vec::new();
        }
        // SAFETY: the caller's contract: `len` readable bytes at `data`.
        unsafe { std::slice::from_raw_parts(data, len as usize).to_vec() }
    }

    /// The version of the WebAuthn API (1 or more where it exists).
    pub(super) fn api_version() -> u32 {
        // SAFETY: takes no arguments and returns a number; it reads and
        // writes no memory of ours. `webauthn.dll` ships with Windows 10
        // 1903 and later.
        unsafe { WebAuthNGetApiVersionNumber() }
    }

    pub(super) fn available() -> bool {
        api_version() >= 1
    }

    /// The window the system dialog belongs to: the active one (the app's,
    /// since the user just connected), or none.
    pub(super) fn owner_window() -> windows_sys::Win32::Foundation::HWND {
        // SAFETY: takes no arguments and returns a window handle or null;
        // it reads and writes no memory of ours.
        unsafe { GetForegroundWindow() }
    }

    /// The system's WebAuthn API.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct WebAuthn;

    impl Authenticator for WebAuthn {
        fn sign(&self, request: &SignRequest) -> Result<Assertion, SkError> {
            let rp_id = wide(&request.application);
            // The API takes mutable pointers but only reads these buffers.
            let mut data = request.data.clone();
            let mut key_handle = request.key_handle.clone();
            let client_data = WEBAUTHN_CLIENT_DATA {
                dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
                cbClientDataJSON: u32::try_from(data.len()).map_err(|_| SkError::Invalid)?,
                pbClientDataJSON: data.as_mut_ptr(),
                pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
            };
            let mut credential = WEBAUTHN_CREDENTIAL {
                dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
                cbId: u32::try_from(key_handle.len()).map_err(|_| SkError::Invalid)?,
                pbId: key_handle.as_mut_ptr(),
                pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            };
            let options = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
                // Version 1: only the fields up to dwFlags are read.
                dwVersion: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_1,
                dwTimeoutMilliseconds: TIMEOUT_MS,
                CredentialList: WEBAUTHN_CREDENTIALS {
                    cCredentials: 1,
                    pCredentials: &mut credential,
                },
                dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
                dwUserVerificationRequirement: if request.flags & USER_VERIFICATION_REQUIRED != 0 {
                    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED
                } else {
                    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED
                },
                ..Default::default()
            };
            let mut assertion: *mut WEBAUTHN_ASSERTION = ptr::null_mut();
            // SAFETY: every pointer points to a live local of this frame for
            // the whole call: `rp_id` is NUL-terminated UTF-16, the client
            // data and the credential point into `data` and `key_handle` with
            // their exact lengths, `options` holds the credential list. The
            // API writes one pointer into `assertion`, which it allocated and
            // `WebAuthNFreeAssertion` frees below.
            let result = unsafe {
                WebAuthNAuthenticatorGetAssertion(
                    owner_window(),
                    rp_id.as_ptr(),
                    &client_data,
                    &options,
                    &mut assertion,
                )
            };
            if result < 0 || assertion.is_null() {
                tracing::info!(code = result, "security key did not sign");
                return Err(super::error_of(result));
            }
            // SAFETY: on success `assertion` points to a WEBAUTHN_ASSERTION of
            // the API whose two buffers hold the stated number of bytes; they
            // are copied before the assertion is freed, once.
            let (auth_data, signature) = unsafe {
                let found = &*assertion;
                let copied = (
                    copy(found.pbAuthenticatorData, found.cbAuthenticatorData),
                    copy(found.pbSignature, found.cbSignature),
                );
                WebAuthNFreeAssertion(assertion);
                copied
            };
            Assertion::from_authenticator_data(&auth_data, signature).ok_or(SkError::Invalid)
        }

        fn create(&self, request: &CreateRequest) -> Result<Attested, SkError> {
            let rp_id = wide(&request.application);
            let user_name = wide("openssh");
            // A credential that is not discoverable: its user id is not
            // used (OpenSSH writes zeros too).
            let mut user_id = [0u8; 32];
            // The challenge is not checked by anyone (no attestation is
            // asked for), but each request gets its own.
            let mut challenge: [u8; 32] =
                shellpp_core::crypto::random_array().map_err(|_| SkError::Failed)?;
            let rp = WEBAUTHN_RP_ENTITY_INFORMATION {
                dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
                pwszId: rp_id.as_ptr(),
                pwszName: rp_id.as_ptr(),
                pwszIcon: ptr::null(),
            };
            let user = WEBAUTHN_USER_ENTITY_INFORMATION {
                dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
                cbId: user_id.len() as u32,
                pbId: user_id.as_mut_ptr(),
                pwszName: user_name.as_ptr(),
                pwszIcon: ptr::null(),
                pwszDisplayName: user_name.as_ptr(),
            };
            // Ed25519 first, ECDSA P-256 for keys without it.
            let mut algorithms =
                [COSE_EDDSA, WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256].map(|algorithm| {
                    WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
                        dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
                        pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
                        lAlg: algorithm,
                    }
                });
            let parameters = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
                cCredentialParameters: algorithms.len() as u32,
                pCredentialParameters: algorithms.as_mut_ptr(),
            };
            let client_data = WEBAUTHN_CLIENT_DATA {
                dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
                cbClientDataJSON: challenge.len() as u32,
                pbClientDataJSON: challenge.as_mut_ptr(),
                pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
            };
            let options = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
                // Version 1: only the fields up to dwFlags are read.
                dwVersion: WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_1,
                dwTimeoutMilliseconds: TIMEOUT_MS,
                dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
                bRequireResidentKey: 0,
                dwUserVerificationRequirement: if request.user_verification {
                    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED
                } else {
                    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED
                },
                dwAttestationConveyancePreference: WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
                ..Default::default()
            };
            let mut attestation: *mut WEBAUTHN_CREDENTIAL_ATTESTATION = ptr::null_mut();
            // SAFETY: every pointer points to a live local of this frame for
            // the whole call: the NUL-terminated `rp_id` and `user_name`, the
            // `user_id`, `algorithms` and `challenge` buffers with their exact
            // lengths. The API writes one pointer into `attestation`, which it
            // allocated and `WebAuthNFreeCredentialAttestation` frees below.
            let result = unsafe {
                WebAuthNAuthenticatorMakeCredential(
                    owner_window(),
                    &rp,
                    &user,
                    &parameters,
                    &client_data,
                    &options,
                    &mut attestation,
                )
            };
            if result < 0 || attestation.is_null() {
                tracing::info!(code = result, "security key made no credential");
                return Err(super::error_of(result));
            }
            // SAFETY: on success `attestation` points to the API's
            // WEBAUTHN_CREDENTIAL_ATTESTATION whose buffers hold the stated
            // number of bytes; they are copied before it is freed, once.
            let (auth_data, key_handle) = unsafe {
                let found = &*attestation;
                let copied = (
                    copy(found.pbAuthenticatorData, found.cbAuthenticatorData),
                    copy(found.pbCredentialId, found.cbCredentialId),
                );
                WebAuthNFreeCredentialAttestation(attestation);
                copied
            };
            if key_handle.is_empty() {
                return Err(SkError::Invalid);
            }
            Ok(Attested {
                key_handle,
                auth_data,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use shellpp_ssh::sk::SkError;

    use super::*;

    #[test]
    fn webauthn_errors_are_mapped() {
        assert_eq!(error_of(0x8009_0036_u32 as i32), SkError::Cancelled);
        assert_eq!(error_of(0x8007_04C7_u32 as i32), SkError::Cancelled);
        assert_eq!(error_of(0x8009_0011_u32 as i32), SkError::UnknownCredential);
        assert_eq!(error_of(0x8009_0035_u32 as i32), SkError::NoAuthenticator);
        assert_eq!(error_of(0x8007_0057_u32 as i32), SkError::Failed);
    }

    #[test]
    fn a_test_run_has_no_authenticator() {
        assert!(authenticator().is_none());
    }

    /// The `unsafe` calls that show no dialog: the API version (Windows
    /// 10 1903 and later have one) and the owner window.
    #[cfg(windows)]
    #[test]
    fn the_webauthn_api_is_there() {
        assert!(windows::api_version() >= 1);
        assert!(windows::available());
        let _ = windows::owner_window();
    }

    /// The signing call with a real key: it shows the system dialog, so it
    /// runs only when a person sets `SHELLPP_TEST_WEBAUTHN=1` and touches a
    /// key (the morning check list); the request names a credential no key
    /// holds, so the answer is "unknown credential" or a cancel.
    #[cfg(windows)]
    #[test]
    fn the_webauthn_sign_call_answers() {
        use shellpp_ssh::sk::{Authenticator as _, SignRequest, SkAlgorithm};
        if std::env::var_os("SHELLPP_TEST_WEBAUTHN").is_none_or(|value| value != "1") {
            return;
        }
        let result = windows::WebAuthn.sign(&SignRequest {
            algorithm: SkAlgorithm::Ed25519,
            application: "ssh:".to_owned(),
            key_handle: vec![0x42; 32],
            flags: 1,
            data: b"shellpp webauthn test".to_vec(),
        });
        assert!(matches!(
            result,
            Err(SkError::UnknownCredential | SkError::Cancelled | SkError::NoAuthenticator)
        ));
    }
}
