//! The `key` record (docs/data-model.md, section 9.1; stage 2 plan, section 4.4).
//!
//! Stage 2 writes only stored keys (`source = stored`) through the short
//! "+ -> Key" path ([`crate::keys`]). Records written by later stages or other
//! clients (`agent` keys, certificates, FIDO2 members, `import_key`) are kept
//! as stored: the core rewrites a key only to add a passphrase, and keeps
//! every member it does not touch.

use serde_json::{Map, Value};

use crate::model::wipe::wipe_value;

/// Dotted paths of the secret members of a key.
pub const SECRET_PATHS: &[&str] = &["private_key", "passphrase"];

/// Longest `label`, in Unicode scalar values.
pub const MAX_LABEL_CHARS: usize = 256;

/// Largest `private_key`, in UTF-8 bytes (`secret(32768)`).
pub const MAX_PRIVATE_KEY_BYTES: usize = 32_768;

/// Largest `passphrase`, in UTF-8 bytes (`secret(1024)`).
pub const MAX_PASSPHRASE_BYTES: usize = 1024;

/// The data of a new stored key. `public_key` is the derived one-line public
/// key; `passphrase` is present only when the user chose to save it.
pub fn new_stored(
    label: &str,
    private_key: &str,
    public_key: &str,
    passphrase: Option<&str>,
) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("label".into(), Value::String(label.to_owned()));
    data.insert("source".into(), Value::String("stored".into()));
    data.insert("private_key".into(), Value::String(private_key.to_owned()));
    data.insert("public_key".into(), Value::String(public_key.to_owned()));
    if let Some(passphrase) = passphrase.filter(|value| !value.is_empty()) {
        data.insert("passphrase".into(), Value::String(passphrase.to_owned()));
    }
    data
}

/// Sets (or, with `None`, removes and wipes) the saved passphrase.
pub fn set_passphrase(data: &mut Map<String, Value>, passphrase: Option<&str>) {
    match passphrase.filter(|value| !value.is_empty()) {
        Some(value) => {
            if let Some(mut old) = data.insert("passphrase".into(), Value::String(value.to_owned()))
            {
                wipe_value(&mut old);
            }
        }
        None => {
            if let Some(mut old) = data.remove("passphrase") {
                wipe_value(&mut old);
            }
        }
    }
}

fn non_empty<'a>(data: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    data.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// The label (empty when missing).
pub fn label(data: &Map<String, Value>) -> &str {
    non_empty(data, "label").unwrap_or("")
}

/// `stored` (the default) or `agent`.
pub fn source(data: &Map<String, Value>) -> &str {
    non_empty(data, "source").unwrap_or("stored")
}

/// The OpenSSH private key text, if stored.
pub fn private_key(data: &Map<String, Value>) -> Option<&str> {
    non_empty(data, "private_key")
}

/// The one-line public key `<algorithm> <base64> [comment]`, if stored.
pub fn public_key(data: &Map<String, Value>) -> Option<&str> {
    non_empty(data, "public_key")
}

/// The saved passphrase, if any.
pub fn passphrase(data: &Map<String, Value>) -> Option<&str> {
    non_empty(data, "passphrase")
}

/// The value of a secret path ([`SECRET_PATHS`]), if set: the Keychain's
/// Show and Copy private key buttons (UI 7.3) read `private_key`.
pub fn secret_value<'a>(data: &'a Map<String, Value>, path: &str) -> Option<&'a str> {
    match path {
        "private_key" => private_key(data),
        "passphrase" => passphrase(data),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_stored_key_data() {
        let data = new_stored("deploy", "PRIVATE", "ssh-ed25519 AAAA c", None);
        assert_eq!(
            Value::Object(data.clone()),
            json!({"label": "deploy", "source": "stored", "private_key": "PRIVATE",
                "public_key": "ssh-ed25519 AAAA c"})
        );
        assert_eq!(label(&data), "deploy");
        assert_eq!(source(&data), "stored");
        assert_eq!(private_key(&data), Some("PRIVATE"));
        assert_eq!(passphrase(&data), None);
        let with = new_stored("k", "P", "ssh-ed25519 AAAA", Some("pp"));
        assert_eq!(passphrase(&with), Some("pp"));
        let empty = new_stored("k", "P", "ssh-ed25519 AAAA", Some(""));
        assert_eq!(passphrase(&empty), None);
    }

    #[test]
    fn passphrase_is_set_and_cleared() {
        let mut data = new_stored("k", "P", "ssh-ed25519 AAAA", None);
        set_passphrase(&mut data, Some("one"));
        assert_eq!(passphrase(&data), Some("one"));
        set_passphrase(&mut data, Some("two"));
        assert_eq!(passphrase(&data), Some("two"));
        set_passphrase(&mut data, None);
        assert_eq!(passphrase(&data), None);
        assert!(!data.contains_key("passphrase"));
    }

    #[test]
    fn secret_values_by_path() {
        let data = new_stored("k", "PRIVATE", "ssh-ed25519 AAAA", Some("pp"));
        assert_eq!(secret_value(&data, "private_key"), Some("PRIVATE"));
        assert_eq!(secret_value(&data, "passphrase"), Some("pp"));
        assert_eq!(secret_value(&data, "public_key"), None);
        assert_eq!(secret_value(&data, "label"), None);
        let without = new_stored("k", "PRIVATE", "ssh-ed25519 AAAA", None);
        assert_eq!(secret_value(&without, "passphrase"), None);
    }

    #[test]
    fn defaults_for_records_of_other_clients() {
        let data = json!({"label": "", "public_key": "ssh-ed25519 AAAA"});
        let data = data.as_object().unwrap();
        assert_eq!(label(data), "");
        assert_eq!(source(data), "stored");
        assert_eq!(private_key(data), None);
    }
}
