//! `SecretStore` on the OS keychain through `keyring` (docs/architecture.md,
//! section 5.2; docs/storage-format.md, section 2.3): service `shellpp`,
//! account `device-key:<db_id>`, value = the 32-byte device key as base64url
//! without padding (43 characters).
//!
//! Availability:
//! - macOS builds without the compile-time constant `SHELLPP_MACOS_SIGNED`
//!   (set by the release workflow for Developer ID signed builds):
//!   `Unavailable(UnsignedBuild)`.
//! - Otherwise a lookup of a probe account that never exists decides: "no
//!   entry" means the keychain works; any other error means it cannot be used
//!   (Linux without a Secret Service provider: `Unavailable(NoSecretService)`).
//!
//! All calls block; the core calls them from `spawn_blocking` threads.

use std::sync::OnceLock;

use secrecy::{ExposeSecret, SecretBox, SecretString};
use shellpp_core::ids;
use shellpp_core::platform::{Availability, SecretStore, SecretStoreError, UnavailableReason};

use super::base64url;

/// Account used only to test whether the keychain answers; never written.
const PROBE_ACCOUNT: &str = "availability-probe";

/// The OS keychain.
#[derive(Debug, Default)]
pub struct KeyringSecretStore {
    availability: OnceLock<Availability>,
}

impl KeyringSecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(account: &str) -> Result<keyring::Entry, SecretStoreError> {
        keyring::Entry::new(ids::KEYCHAIN_SERVICE, account).map_err(map_error)
    }

    fn probe() -> Availability {
        if cfg!(target_os = "macos") && option_env!("SHELLPP_MACOS_SIGNED").is_none() {
            return Availability::Unavailable(UnavailableReason::UnsignedBuild);
        }
        let unavailable = if cfg!(target_os = "linux") {
            UnavailableReason::NoSecretService
        } else {
            UnavailableReason::Unsupported
        };
        let entry = match keyring::Entry::new(ids::KEYCHAIN_SERVICE, PROBE_ACCOUNT) {
            Ok(entry) => entry,
            Err(_) => return Availability::Unavailable(unavailable),
        };
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Availability::Available,
            Err(error) => {
                tracing::info!(kind = error_kind(&error), "OS keychain unavailable");
                Availability::Unavailable(unavailable)
            }
        }
    }
}

/// A data-free name for a keyring error (log lines only).
fn error_kind(error: &keyring::Error) -> &'static str {
    match error {
        keyring::Error::PlatformFailure(_) => "platform_failure",
        keyring::Error::NoStorageAccess(_) => "no_storage_access",
        keyring::Error::NoEntry => "no_entry",
        keyring::Error::BadEncoding(_) => "bad_encoding",
        keyring::Error::TooLong(..) => "too_long",
        keyring::Error::Invalid(..) => "invalid",
        keyring::Error::Ambiguous(_) => "ambiguous",
        _ => "other",
    }
}

fn map_error(error: keyring::Error) -> SecretStoreError {
    match error {
        keyring::Error::NoStorageAccess(_) => SecretStoreError::Unavailable,
        keyring::Error::BadEncoding(_) => SecretStoreError::Malformed,
        // Platform messages can name the account; keep only the kind.
        other => SecretStoreError::Backend(error_kind(&other).to_owned()),
    }
}

impl SecretStore for KeyringSecretStore {
    fn availability(&self) -> Availability {
        *self.availability.get_or_init(Self::probe)
    }

    fn get(&self, account: &str) -> Result<Option<SecretBox<[u8; 32]>>, SecretStoreError> {
        if self.availability() != Availability::Available {
            return Err(SecretStoreError::Unavailable);
        }
        let text = match Self::entry(account)?.get_password() {
            Ok(text) => SecretString::from(text),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(map_error(error)),
        };
        let key = base64url::decode_key(text.expose_secret()).ok_or(SecretStoreError::Malformed)?;
        // Copied straight into the box: no unwiped copy on the stack.
        Ok(Some(SecretBox::init_with_mut(|buf: &mut [u8; 32]| {
            buf.copy_from_slice(&key[..]);
        })))
    }

    fn set(&self, account: &str, secret: &SecretBox<[u8; 32]>) -> Result<(), SecretStoreError> {
        if self.availability() != Availability::Available {
            return Err(SecretStoreError::Unavailable);
        }
        let text = base64url::encode(secret.expose_secret());
        Self::entry(account)?.set_password(&text).map_err(map_error)
    }

    fn delete(&self, account: &str) -> Result<(), SecretStoreError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_carry_no_data() {
        assert_eq!(error_kind(&keyring::Error::NoEntry), "no_entry");
        let backend = map_error(keyring::Error::Invalid(
            "user".to_owned(),
            "device-key:secret-looking".to_owned(),
        ));
        assert_eq!(backend.to_string(), "secret store failure: invalid");
        assert!(matches!(
            map_error(keyring::Error::BadEncoding(vec![1, 2])),
            SecretStoreError::Malformed
        ));
    }

    /// Writes, reads and deletes a random test account in the real OS
    /// keychain. Not run by default: `cargo test -p shellpp-desktop -- --ignored`.
    #[test]
    #[ignore = "touches the OS keychain"]
    fn round_trip_in_the_os_keychain() {
        let store = KeyringSecretStore::new();
        if cfg!(windows) {
            // The Windows Credential Manager is always present.
            assert_eq!(store.availability(), Availability::Available);
        }
        if store.availability() != Availability::Available {
            return;
        }
        let account = format!("test-{}", uuid::Uuid::now_v7());
        let key = SecretBox::new(Box::new([0x5a_u8; 32]));
        store.set(&account, &key).unwrap();
        let read = store.get(&account).unwrap().unwrap();
        assert_eq!(read.expose_secret(), key.expose_secret());
        store.delete(&account).unwrap();
        assert!(store.get(&account).unwrap().is_none());
        store.delete(&account).unwrap();
    }
}
