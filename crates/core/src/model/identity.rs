//! The `identity` record (docs/data-model.md, section 9.4; UI 3.1 "+ ->
//! Identity", 7.4): a username plus a password and/or a key, reused across
//! hosts.
//!
//! [`build`] validates the members the app edits (`label` required,
//! `username`, `key_id`), keeps unknown members of the input and of the stored
//! data, keeps the managed `import_key`, and applies the password change. The
//! password is a secret member: it never travels inside the input data.

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{Errors, opt_string, opt_uuid, present};
use crate::model::merge::{keep_from, merge_level};
use crate::model::text;
use crate::model::wipe::{WipeOnDrop, wipe_value};

/// Dotted paths of the secret members of an identity.
pub const SECRET_PATHS: &[&str] = &["password"];

/// Members only the core writes; input values are ignored.
pub const MANAGED: &[&str] = &["import_key"];

const KNOWN: &[&str] = &["label", "username", "password", "key_id", "import_key"];

/// Longest `label` and `username`, in Unicode scalar values.
pub const MAX_LABEL_CHARS: usize = 256;
pub const MAX_USERNAME_CHARS: usize = 256;

/// Largest `password`, in UTF-8 bytes (`secret(4096)`).
pub const MAX_PASSWORD_BYTES: usize = 4096;

/// What a save does with the stored password.
#[derive(Clone, Debug, Default)]
pub enum PasswordChange {
    /// Keep the stored value.
    #[default]
    Keep,
    /// Store this value (an empty value clears it).
    Set(SecretString),
    /// Remove the stored value.
    Clear,
}

/// Validates `input` and builds the data to store. Errors use the member
/// names: `label` (`required`, `too_long`, `invalid`), `username`, `key_id`,
/// `password` (`not_allowed` inside the input, `too_long`, `invalid`).
pub fn build(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
    password: &PasswordChange,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    if present(input, "password").is_some() {
        errors.push("password", ValidationReason::NotAllowed);
    }
    let mut normalized = Map::new();
    match present(input, "label") {
        None => errors.push("label", ValidationReason::Required),
        Some(value) => match value.as_str().map(|raw| text::string(raw, MAX_LABEL_CHARS)) {
            Some(Ok(Some(label))) => {
                normalized.insert("label".into(), Value::String(label));
            }
            Some(Ok(None)) => errors.push("label", ValidationReason::Required),
            Some(Err(reason)) => errors.push("label", reason),
            None => errors.push("label", ValidationReason::Invalid),
        },
    }
    if let Some(username) = opt_string(
        input,
        "username",
        "username",
        MAX_USERNAME_CHARS,
        &mut errors,
    ) {
        normalized.insert("username".into(), Value::String(username));
    }
    if let Some(key_id) = opt_uuid(input, "key_id", "key_id", &mut errors) {
        normalized.insert("key_id".into(), json!(key_id));
    }
    if let PasswordChange::Set(value) = password
        && let Err(reason) = text::secret(value.expose_secret(), MAX_PASSWORD_BYTES)
    {
        errors.push("password", reason);
    }
    if !errors.is_empty() {
        return Err(errors.into_vec());
    }

    // Holds the stored password: wiped on every path.
    let mut data = WipeOnDrop::new(merge_level(KNOWN, normalized, Some(input), stored));
    keep_from(&mut data, stored, MANAGED);
    keep_from(&mut data, stored, SECRET_PATHS);
    let value = match password {
        PasswordChange::Keep => return Ok(data.into_inner()),
        PasswordChange::Set(value) => Some(value.expose_secret()).filter(|value| !value.is_empty()),
        PasswordChange::Clear => None,
    };
    match value {
        Some(value) => {
            if let Some(mut old) = data.insert("password".into(), Value::String(value.to_owned())) {
                wipe_value(&mut old);
            }
        }
        None => {
            if let Some(mut old) = data.remove("password") {
                wipe_value(&mut old);
            }
        }
    }
    Ok(data.into_inner())
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

/// The username, if set.
pub fn username(data: &Map<String, Value>) -> Option<&str> {
    non_empty(data, "username")
}

/// The stored password, if set.
pub fn password(data: &Map<String, Value>) -> Option<&str> {
    non_empty(data, "password")
}

/// The linked key, if set and well-formed.
pub fn key_id(data: &Map<String, Value>) -> Option<uuid::Uuid> {
    non_empty(data, "key_id").and_then(|raw| uuid::Uuid::parse_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationReason::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn reasons(
        result: Result<Map<String, Value>, Vec<FieldError>>,
    ) -> Vec<(String, ValidationReason)> {
        result
            .unwrap_err()
            .into_iter()
            .map(|error| (error.field, error.reason))
            .collect()
    }

    #[test]
    fn a_new_identity_needs_a_label() {
        assert_eq!(
            reasons(build(&map(json!({})), None, &PasswordChange::Keep)),
            vec![("label".into(), Required)]
        );
        assert_eq!(
            reasons(build(
                &map(json!({"label": "  "})),
                None,
                &PasswordChange::Keep
            )),
            vec![("label".into(), Required)]
        );
        let data = build(
            &map(json!({"label": " deploy ", "username": "ops"})),
            None,
            &PasswordChange::Set(SecretString::from("p4ss")),
        )
        .unwrap();
        assert_eq!(label(&data), "deploy");
        assert_eq!(username(&data), Some("ops"));
        assert_eq!(password(&data), Some("p4ss"));
    }

    #[test]
    fn invalid_members_and_secret_paths_are_refused() {
        let input = map(json!({
            "label": "x".repeat(257),
            "username": "u\u{7}",
            "key_id": "nope",
            "password": "in data"
        }));
        assert_eq!(
            reasons(build(
                &input,
                None,
                &PasswordChange::Set(SecretString::from("x".repeat(4097)))
            )),
            vec![
                ("password".into(), NotAllowed),
                ("label".into(), TooLong),
                ("username".into(), Invalid),
                ("key_id".into(), Invalid),
                ("password".into(), TooLong),
            ]
        );
    }

    #[test]
    fn password_is_kept_set_and_cleared_and_unknown_members_survive() {
        let stored = map(json!({
            "label": "old",
            "password": "secret",
            "import_key": "ssh_config:x",
            "x_future": 1
        }));
        let kept = build(
            &map(json!({"label": "new", "import_key": "ignored"})),
            Some(&stored),
            &PasswordChange::Keep,
        )
        .unwrap();
        assert_eq!(
            Value::Object(kept.clone()),
            json!({"label": "new", "password": "secret", "import_key": "ssh_config:x", "x_future": 1})
        );
        let cleared = build(
            &map(json!({"label": "new"})),
            Some(&kept),
            &PasswordChange::Clear,
        )
        .unwrap();
        assert_eq!(password(&cleared), None);
        let empty = build(
            &map(json!({"label": "new"})),
            Some(&kept),
            &PasswordChange::Set(SecretString::from("")),
        )
        .unwrap();
        assert_eq!(password(&empty), None);
    }
}
