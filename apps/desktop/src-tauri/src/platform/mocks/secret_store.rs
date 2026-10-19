//! In-memory keychain persisted to `<data_dir>/e2e-keychain.json` (e2e
//! builds only, debug builds only). The file holds the availability flag and
//! the stored items as base64url text, like the real keychain encoding.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretBox};
use serde::{Deserialize, Serialize};
use shellpp_core::platform::{Availability, SecretStore, SecretStoreError, UnavailableReason};

use crate::platform::base64url;

/// File name inside the data directory.
pub const FILE_NAME: &str = "e2e-keychain.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    available: bool,
    #[serde(default)]
    items: BTreeMap<String, String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            available: true,
            items: BTreeMap::new(),
        }
    }
}

/// The e2e keychain.
pub struct FileSecretStore {
    path: PathBuf,
    state: Mutex<State>,
}

impl std::fmt::Debug for FileSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileSecretStore { .. }")
    }
}

impl FileSecretStore {
    /// Loads the file if it exists (a missing or unreadable file starts empty and available).
    pub fn open(path: PathBuf) -> Self {
        let state = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn persist(&self, state: &State) -> Result<(), SecretStoreError> {
        let json = serde_json::to_vec_pretty(state)
            .map_err(|_| SecretStoreError::Backend("encode".to_owned()))?;
        std::fs::write(&self.path, json).map_err(|_| SecretStoreError::Backend("write".to_owned()))
    }

    /// `e2e_mock {kind: "secret_store_set_available"}`.
    pub fn set_available(&self, available: bool) -> Result<(), SecretStoreError> {
        let mut state = self.lock();
        state.available = available;
        self.persist(&state)
    }

    /// `e2e_mock {kind: "secret_store_accounts"}`: account names only.
    pub fn accounts(&self) -> Vec<String> {
        self.lock().items.keys().cloned().collect()
    }
}

impl SecretStore for FileSecretStore {
    fn availability(&self) -> Availability {
        if self.lock().available {
            Availability::Available
        } else {
            Availability::Unavailable(UnavailableReason::Unsupported)
        }
    }

    fn get(&self, account: &str) -> Result<Option<SecretBox<[u8; 32]>>, SecretStoreError> {
        let state = self.lock();
        if !state.available {
            return Err(SecretStoreError::Unavailable);
        }
        match state.items.get(account) {
            None => Ok(None),
            Some(text) => {
                let key = base64url::decode_key(text).ok_or(SecretStoreError::Malformed)?;
                Ok(Some(SecretBox::init_with_mut(|buf: &mut [u8; 32]| {
                    buf.copy_from_slice(&key[..]);
                })))
            }
        }
    }

    fn set(&self, account: &str, secret: &SecretBox<[u8; 32]>) -> Result<(), SecretStoreError> {
        let mut state = self.lock();
        if !state.available {
            return Err(SecretStoreError::Unavailable);
        }
        let text = base64url::encode(secret.expose_secret());
        state.items.insert(account.to_owned(), text.to_string());
        self.persist(&state)
    }

    fn delete(&self, account: &str) -> Result<(), SecretStoreError> {
        let mut state = self.lock();
        state.items.remove(account);
        self.persist(&state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_items_and_availability() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        let store = FileSecretStore::open(path.clone());
        assert_eq!(store.availability(), Availability::Available);
        let key = SecretBox::new(Box::new([7u8; 32]));
        store.set("device-key:x", &key).unwrap();
        assert_eq!(store.accounts(), vec!["device-key:x".to_owned()]);

        let reopened = FileSecretStore::open(path.clone());
        let read = reopened.get("device-key:x").unwrap().unwrap();
        assert_eq!(read.expose_secret(), &[7u8; 32]);
        assert!(reopened.get("device-key:y").unwrap().is_none());

        reopened.set_available(false).unwrap();
        let again = FileSecretStore::open(path);
        assert_eq!(
            again.availability(),
            Availability::Unavailable(UnavailableReason::Unsupported)
        );
        assert!(matches!(
            again.get("device-key:x"),
            Err(SecretStoreError::Unavailable)
        ));
        again.delete("device-key:x").unwrap();
        assert!(again.accounts().is_empty());
    }
}
