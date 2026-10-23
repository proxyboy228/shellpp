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

/// Whether this system can make a new key on a security key.
pub fn can_create() -> bool {
    authenticator().is_some()
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

/// A failure `HRESULT` (winerror.h): the severity bit, the facility and
/// the code.
#[cfg(any(windows, test))]
const fn failure(facility: u32, code: u32) -> u32 {
    (1 << 31) | (facility << 16) | code
}

/// The facilities of the WebAuthn errors: Win32 errors and NTE_* (SSPI).
#[cfg(any(windows, test))]
const FACILITY_WIN32: u32 = 7;
#[cfg(any(windows, test))]
const FACILITY_SSPI: u32 = 9;
#[cfg(any(windows, test))]
const NTE_NOT_FOUND: u32 = failure(FACILITY_SSPI, 0x11);
#[cfg(any(windows, test))]
const NTE_DEVICE_NOT_FOUND: u32 = failure(FACILITY_SSPI, 0x35);
#[cfg(any(windows, test))]
const NTE_USER_CANCELLED: u32 = failure(FACILITY_SSPI, 0x36);
/// HRESULT_FROM_WIN32(ERROR_CANCELLED).
#[cfg(any(windows, test))]
const CANCELLED: u32 = failure(FACILITY_WIN32, 1223);
/// HRESULT_FROM_WIN32(ERROR_TIMEOUT).
#[cfg(any(windows, test))]
const TIMED_OUT: u32 = failure(FACILITY_WIN32, 1460);

/// What an `HRESULT` of the WebAuthn API means for the sign-in.
#[cfg(any(windows, test))]
fn error_of(hresult: i32) -> shellpp_ssh::sk::SkError {
    use shellpp_ssh::sk::SkError;
    // The bit pattern of an HRESULT: compare as unsigned.
    match hresult as u32 {
        NTE_USER_CANCELLED | CANCELLED | TIMED_OUT => SkError::Cancelled,
        // The key does not hold the credential.
        NTE_NOT_FOUND => SkError::UnknownCredential,
        NTE_DEVICE_NOT_FOUND => SkError::NoAuthenticator,
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
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Networking::WindowsWebServices::{
        WEBAUTHN_ASSERTION, WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
        WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
        WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_1, WEBAUTHN_CLIENT_DATA,
        WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, WEBAUTHN_CREDENTIAL,
        WEBAUTHN_CREDENTIAL_CURRENT_VERSION, WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
        WEBAUTHN_CREDENTIALS, WEBAUTHN_HASH_ALGORITHM_SHA_256,
        WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
        WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
    };
    use windows_sys::Win32::Networking::WindowsWebServices::{
        WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS,
        WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_1,
        WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256, WEBAUTHN_COSE_CREDENTIAL_PARAMETER,
        WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION, WEBAUTHN_COSE_CREDENTIAL_PARAMETERS,
        WEBAUTHN_CREDENTIAL_ATTESTATION, WEBAUTHN_RP_ENTITY_INFORMATION,
        WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION, WEBAUTHN_USER_ENTITY_INFORMATION,
        WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    use windows_sys::core::{HRESULT, PCWSTR};

    type GetApiVersionNumber = unsafe extern "system" fn() -> u32;
    type GetAssertion = unsafe extern "system" fn(
        HWND,
        PCWSTR,
        *const WEBAUTHN_CLIENT_DATA,
        *const WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
        *mut *mut WEBAUTHN_ASSERTION,
    ) -> HRESULT;
    type FreeAssertion = unsafe extern "system" fn(*const WEBAUTHN_ASSERTION);
    type MakeCredential = unsafe extern "system" fn(
        HWND,
        *const WEBAUTHN_RP_ENTITY_INFORMATION,
        *const WEBAUTHN_USER_ENTITY_INFORMATION,
        *const WEBAUTHN_COSE_CREDENTIAL_PARAMETERS,
        *const WEBAUTHN_CLIENT_DATA,
        *const WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS,
        *mut *mut WEBAUTHN_CREDENTIAL_ATTESTATION,
    ) -> HRESULT;
    type FreeCredentialAttestation =
        unsafe extern "system" fn(*const WEBAUTHN_CREDENTIAL_ATTESTATION);

    /// The functions of `webauthn.dll`, loaded at run time from System32:
    /// a load-time import would keep the app from starting where the DLL
    /// does not exist (Windows before 10 1903).
    pub(super) struct Api {
        get_api_version_number: GetApiVersionNumber,
        get_assertion: GetAssertion,
        free_assertion: FreeAssertion,
        make_credential: MakeCredential,
        free_credential_attestation: FreeCredentialAttestation,
    }

    /// `webauthn.dll`, loaded once and never unloaded; `None` without it.
    pub(super) fn api() -> Option<&'static Api> {
        static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();
        API.get_or_init(load).as_ref()
    }

    fn load() -> Option<Api> {
        let name = wide("webauthn.dll");
        // SAFETY: `name` is a NUL-terminated UTF-16 string that lives through
        // the call; the search is limited to System32, so no other folder's
        // DLL of that name is loaded. The module stays loaded for the life
        // of the process (never freed), so the function pointers below
        // stay valid.
        let module =
            unsafe { LoadLibraryExW(name.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return None;
        }
        /// A function of the loaded module by its NUL-terminated name.
        macro_rules! function {
            ($name:literal, $type:ty) => {{
                // SAFETY: `module` is the loaded webauthn.dll and the name is
                // NUL-terminated ASCII. The address is that of the exported
                // function named, whose signature in webauthn.h is `$type`;
                // a missing export is `None`.
                let address = unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) }?;
                // SAFETY: both are function pointers of the same size; the
                // export has exactly the signature `$type`.
                unsafe {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, $type>(address)
                }
            }};
        }
        Some(Api {
            get_api_version_number: function!("WebAuthNGetApiVersionNumber", GetApiVersionNumber),
            get_assertion: function!("WebAuthNAuthenticatorGetAssertion", GetAssertion),
            free_assertion: function!("WebAuthNFreeAssertion", FreeAssertion),
            make_credential: function!("WebAuthNAuthenticatorMakeCredential", MakeCredential),
            free_credential_attestation: function!(
                "WebAuthNFreeCredentialAttestation",
                FreeCredentialAttestation
            ),
        })
    }

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

    /// The version of the WebAuthn API: 1 or more where it exists, 0
    /// without `webauthn.dll`.
    pub(super) fn api_version() -> u32 {
        let Some(api) = api() else {
            return 0;
        };
        // SAFETY: takes no arguments and returns a number; it reads and
        // writes no memory of ours.
        unsafe { (api.get_api_version_number)() }
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
            let api = api().ok_or(SkError::NoAuthenticator)?;
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
                (api.get_assertion)(
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
                (api.free_assertion)(assertion);
                copied
            };
            Assertion::from_authenticator_data(&auth_data, signature).ok_or(SkError::Invalid)
        }

        fn create(&self, request: &CreateRequest) -> Result<Attested, SkError> {
            let api = api().ok_or(SkError::NoAuthenticator)?;
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
                (api.make_credential)(
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
                (api.free_credential_attestation)(attestation);
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
        // The values winerror.h gives them.
        assert_eq!(NTE_USER_CANCELLED, 2_148_073_526);
        assert_eq!(CANCELLED, 2_147_943_623);
        assert_eq!(NTE_NOT_FOUND, 2_148_073_489);
        assert_eq!(error_of(NTE_USER_CANCELLED as i32), SkError::Cancelled);
        assert_eq!(error_of(CANCELLED as i32), SkError::Cancelled);
        assert_eq!(error_of(TIMED_OUT as i32), SkError::Cancelled);
        assert_eq!(error_of(NTE_NOT_FOUND as i32), SkError::UnknownCredential);
        assert_eq!(
            error_of(NTE_DEVICE_NOT_FOUND as i32),
            SkError::NoAuthenticator
        );
        // E_INVALIDARG.
        assert_eq!(
            error_of(failure(FACILITY_WIN32, 87) as i32),
            SkError::Failed
        );
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

    /// The making call with a real key, as above (`SHELLPP_TEST_WEBAUTHN=1`):
    /// a touch makes a credential whose key file is read back; a cancel is
    /// an answer too.
    #[cfg(windows)]
    #[test]
    fn the_webauthn_create_call_answers() {
        use shellpp_ssh::sk::{Authenticator as _, CreateRequest, key_file};
        if std::env::var_os("SHELLPP_TEST_WEBAUTHN").is_none_or(|value| value != "1") {
            return;
        }
        let request = CreateRequest {
            application: "ssh:".to_owned(),
            user_verification: false,
        };
        match windows::WebAuthn.create(&request) {
            Ok(attested) => assert!(key_file(&request, &attested, "webauthn test").is_ok()),
            Err(error) => assert!(matches!(
                error,
                SkError::Cancelled | SkError::NoAuthenticator
            )),
        }
    }
}
