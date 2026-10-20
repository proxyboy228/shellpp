//! Keychain records written from the host panel's "+" and the Keychain
//! screen (UI 3.1, 7.3, 7.4; docs/data-model.md, sections 9.1, 9.2 and
//! 9.4): identities (list, create, edit), the OpenSSH user certificate
//! stored on a key, a key's label and its public key.
//!
//! These methods are facade methods like the ones in `service/mod.rs`; they
//! live here so the Keychain additions of stage 1 stay in one place. Every
//! write goes through the record path (seal, one transaction, index,
//! `records:changed` with the `keys` class). Nothing here logs labels, user
//! names, passwords, keys or certificates.

use std::fmt;

use secrecy::SecretString;
use serde_json::{Map, Value, json};
use ssh_key::certificate::CertType;
use ssh_key::{Certificate, PrivateKey, PublicKey};
use uuid::Uuid;

use super::Core;
use super::dto::{KeyView, RecordRef};
use super::records::{check_vault, parts};
use super::session::Session;
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::keys;
use crate::model::identity::{self, PasswordChange};
use crate::model::key;
use crate::model::text::{self, fold};
use crate::model::time::{format_unix_ms, uuid_v7_timestamp};
use crate::model::types::RecordKind;
use crate::model::wipe::WipeOnDrop;

/// Longest `key.certificate` line (`string(16384)`, docs/data-model.md, 9.1).
pub const MAX_CERTIFICATE_CHARS: usize = 16_384;

/// An `identity` record without its password (UI 3.1, 7.2, 7.4).
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityView {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub created_at: String,
    pub modified_at: String,
    pub label: String,
    pub username: Option<String>,
    pub key_id: Option<Uuid>,
    /// A password is stored.
    pub password_set: bool,
}

impl fmt::Debug for IdentityView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityView")
            .field("record_id", &self.record_id)
            .field("password_set", &self.password_set)
            .finish_non_exhaustive()
    }
}

/// Input of [`Core::identity_save`]: `record_id` `None` creates an identity.
/// `label` is required; an empty `username` removes it.
pub struct IdentitySaveInput {
    pub vault_id: Uuid,
    pub record_id: Option<Uuid>,
    pub label: String,
    pub username: String,
    pub key_id: Option<Uuid>,
    pub password: PasswordChange,
}

impl fmt::Debug for IdentitySaveInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentitySaveInput")
            .field("record_id", &self.record_id)
            .finish_non_exhaustive()
    }
}

/// Input of [`Core::key_certificate_set`]: one OpenSSH certificate line for
/// the key `key`; an empty text removes the stored certificate.
pub struct KeyCertificateInput {
    pub key: RecordRef,
    pub certificate: String,
}

impl fmt::Debug for KeyCertificateInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyCertificateInput")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Input of [`Core::key_label_set`]: the new label of the key `key`
/// (`string(256)`, required).
pub struct KeyLabelInput {
    pub key: RecordRef,
    pub label: String,
}

impl fmt::Debug for KeyLabelInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyLabelInput")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// The one-line public key of a key record: its `public_key` member, else
/// derived from the private key (the public half of an OpenSSH key is
/// readable without its passphrase). `None` when neither can be read.
pub fn public_key_line(data: &Map<String, Value>) -> Option<String> {
    if let Some(line) = key::public_key(data) {
        return Some(line.to_owned());
    }
    let private = PrivateKey::from_openssh(key::private_key(data)?.as_bytes()).ok()?;
    private.public_key().to_openssh().ok()
}

fn key_view(session: &Session, record_id: Uuid) -> Result<KeyView, CoreError> {
    let record = session
        .index
        .get_kind(&record_id, RecordKind::Key)
        .ok_or(CoreError::NotFound)?;
    Ok(keys::view(
        session.ring.vault_id(),
        record_id,
        record.payload.hlc.ms(),
        record.data(),
    ))
}

fn identity_view(session: &Session, record_id: Uuid) -> Result<IdentityView, CoreError> {
    let record = session
        .index
        .get_kind(&record_id, RecordKind::Identity)
        .ok_or(CoreError::NotFound)?;
    let data = record.data();
    Ok(IdentityView {
        vault_id: session.ring.vault_id(),
        record_id,
        created_at: uuid_v7_timestamp(&record_id),
        modified_at: format_unix_ms(record.payload.hlc.ms()),
        label: identity::label(data).to_owned(),
        username: identity::username(data).map(str::to_owned),
        key_id: identity::key_id(data),
        password_set: identity::password(data).is_some(),
    })
}

/// The public key a key record certifies: its `public_key` line, else the
/// public half of its private key (readable without the passphrase).
fn key_public_data(data: &Map<String, Value>) -> Option<ssh_key::public::KeyData> {
    if let Some(line) = key::public_key(data)
        && let Ok(public) = PublicKey::from_openssh(line)
    {
        return Some(public.key_data().clone());
    }
    key::private_key(data)
        .and_then(|text| PrivateKey::from_openssh(text.as_bytes()).ok())
        .map(|private| private.public_key().key_data().clone())
}

/// Checks one certificate line against a key (docs/data-model.md, 9.2):
/// `required` (empty), `too_long`, `invalid` (not one OpenSSH certificate),
/// `not_allowed` (a host certificate, or one for another public key).
pub fn check_certificate(
    raw: &str,
    key_data: &Map<String, Value>,
) -> Result<Option<String>, FieldError> {
    let refuse = |reason| FieldError::new("certificate", reason);
    let line = raw.trim();
    if line.is_empty() {
        return Ok(None);
    }
    if line.chars().count() > MAX_CERTIFICATE_CHARS {
        return Err(refuse(ValidationReason::TooLong));
    }
    if line.contains(['\n', '\r', '\0']) {
        return Err(refuse(ValidationReason::Invalid));
    }
    let certificate =
        Certificate::from_openssh(line).map_err(|_| refuse(ValidationReason::Invalid))?;
    if certificate.cert_type() != CertType::User {
        return Err(refuse(ValidationReason::NotAllowed));
    }
    match key_public_data(key_data) {
        Some(public) if &public == certificate.public_key() => {}
        _ => return Err(refuse(ValidationReason::NotAllowed)),
    }
    Ok(Some(line.to_owned()))
}

impl Core {
    /// Every identity of the personal vault, by label (UI 3.1 "+ -> Identity").
    pub fn identities_list(&self) -> Result<Vec<IdentityView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let mut views: Vec<IdentityView> = session
            .index
            .of_kind(RecordKind::Identity)
            .filter_map(|record| identity_view(session, record.record_id).ok())
            .collect();
        views.sort_by(|a, b| {
            fold(&a.label)
                .cmp(&fold(&b.label))
                .then_with(|| a.record_id.cmp(&b.record_id))
        });
        Ok(views)
    }

    /// Creates or updates an identity (docs/data-model.md, 9.4). `key_id`
    /// must name a live key (`missing_ref`). Members the input does not name
    /// are kept.
    pub fn identity_save(&self, input: IdentitySaveInput) -> Result<IdentityView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        let stored: Option<WipeOnDrop> = match input.record_id {
            Some(id) => Some(WipeOnDrop::new(
                parts
                    .session
                    .index
                    .get_kind(&id, RecordKind::Identity)
                    .ok_or(CoreError::NotFound)?
                    .data()
                    .clone(),
            )),
            None => None,
        };
        let mut data = Map::new();
        data.insert("label".into(), Value::String(input.label.clone()));
        data.insert(
            "username".into(),
            match input.username.trim() {
                "" => Value::Null,
                username => Value::String(username.to_owned()),
            },
        );
        data.insert(
            "key_id".into(),
            input.key_id.map_or(Value::Null, |id| json!(id)),
        );
        let mut errors = Vec::new();
        if let Some(key_id) = input.key_id
            && parts
                .session
                .index
                .get_kind(&key_id, RecordKind::Key)
                .is_none()
        {
            errors.push(FieldError::new("key_id", ValidationReason::MissingRef));
        }
        let built = identity::build(&data, stored.as_deref(), &input.password);
        let built = match built {
            Ok(built) if errors.is_empty() => built,
            Ok(built) => {
                drop(WipeOnDrop::new(built));
                return Err(CoreError::Validation(errors));
            }
            Err(mut refused) => {
                refused.extend(errors);
                return Err(CoreError::Validation(refused));
            }
        };
        let record_id = input.record_id.unwrap_or_else(|| parts.ids.next(now));
        let write = parts.session.prepare_write(
            parts.store,
            RecordKind::Identity,
            record_id,
            built,
            now,
        )?;
        self.commit_records(&mut parts, vec![write], Vec::new(), Vec::new())?;
        tracing::info!(record_id = %record_id, created = input.record_id.is_none(), "identity saved");
        identity_view(parts.session, record_id)
    }

    /// The stored password of an identity (UI 7.4 show button; connect time).
    pub fn identity_password(&self, identity: RecordRef) -> Result<SecretString, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(identity.vault_id))?;
        let record = session
            .index
            .get_kind(&identity.record_id, RecordKind::Identity)
            .ok_or(CoreError::NotFound)?;
        identity::password(record.data())
            .map(|value| SecretString::from(value.to_owned()))
            .ok_or(CoreError::NotFound)
    }

    /// Stores (or with an empty text removes) the user certificate of a key
    /// (UI 3.1 "+ -> Certificate"; docs/data-model.md, 9.2). The certificate
    /// must be a user certificate for exactly this key's public key.
    pub fn key_certificate_set(&self, input: KeyCertificateInput) -> Result<KeyView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.key.vault_id))?;
        let record = parts
            .session
            .index
            .get_kind(&input.key.record_id, RecordKind::Key)
            .ok_or(CoreError::NotFound)?;
        let mut data = WipeOnDrop::new(record.data().clone());
        let certificate = check_certificate(&input.certificate, &data)
            .map_err(|error| CoreError::Validation(vec![error]))?;
        match certificate {
            Some(line) => {
                data.insert("certificate".into(), Value::String(line));
            }
            None => {
                data.remove("certificate");
            }
        }
        let write = parts.session.prepare_write(
            parts.store,
            RecordKind::Key,
            input.key.record_id,
            data.into_inner(),
            now,
        )?;
        self.commit_records(&mut parts, vec![write], Vec::new(), Vec::new())?;
        tracing::info!(record_id = %input.key.record_id, "key certificate saved");
        let record = parts
            .session
            .index
            .get_kind(&input.key.record_id, RecordKind::Key)
            .ok_or(CoreError::Internal("key missing"))?;
        Ok(keys::view(
            parts.session.ring.vault_id(),
            input.key.record_id,
            record.payload.hlc.ms(),
            record.data(),
        ))
    }

    /// Whether a key holds a certificate (the host panel's certificate row).
    pub fn key_has_certificate(&self, key: RecordRef) -> Result<bool, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(key.vault_id))?;
        let record = session
            .index
            .get_kind(&key.record_id, RecordKind::Key)
            .ok_or(CoreError::NotFound)?;
        Ok(record
            .data()
            .get("certificate")
            .and_then(Value::as_str)
            .is_some_and(|line| !line.is_empty()))
    }

    /// Renames a key (the Keychain's key panel, UI 7.3; saved as in UI 1.3):
    /// `label` `required` when empty, `too_long` over 256 characters. Every
    /// other member is kept; an unchanged label writes nothing.
    pub fn key_label_set(&self, input: KeyLabelInput) -> Result<KeyView, CoreError> {
        let label = match text::string(&input.label, key::MAX_LABEL_CHARS) {
            Ok(Some(label)) => label,
            Ok(None) => return Err(CoreError::field("label", ValidationReason::Required)),
            Err(reason) => return Err(CoreError::field("label", reason)),
        };
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.key.vault_id))?;
        let record_id = input.key.record_id;
        let record = parts
            .session
            .index
            .get_kind(&record_id, RecordKind::Key)
            .ok_or(CoreError::NotFound)?;
        if key::label(record.data()) == label {
            return key_view(parts.session, record_id);
        }
        let mut data = WipeOnDrop::new(record.data().clone());
        data.insert("label".into(), Value::String(label));
        let write = parts.session.prepare_write(
            parts.store,
            RecordKind::Key,
            record_id,
            data.into_inner(),
            now,
        )?;
        self.commit_records(&mut parts, vec![write], Vec::new(), Vec::new())?;
        tracing::info!(record_id = %record_id, "key label saved");
        key_view(parts.session, record_id)
    }

    /// The one-line public key of a key (UI 7.3: shown in the key panel,
    /// derived from the private key when the record has none). Not a
    /// secret (docs/security.md, section 9.4). `not_found` when the key
    /// cannot be read.
    pub fn key_public_key(&self, key: RecordRef) -> Result<String, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(key.vault_id))?;
        let record = session
            .index
            .get_kind(&key.record_id, RecordKind::Key)
            .ok_or(CoreError::NotFound)?;
        public_key_line(record.data()).ok_or(CoreError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::generate::{KeyKind, generate};

    /// A record without `public_key` (written by another client) still
    /// shows its public key: the public half of the private key, also for
    /// an encrypted key.
    #[test]
    fn the_public_key_is_derived_when_the_record_has_none() {
        let passphrase = SecretString::from(format!(
            "pp-{:x}",
            u64::from_le_bytes(crate::crypto::random_array().expect("random"))
        ));
        for passphrase in [None, Some(&passphrase)] {
            let text = generate(KeyKind::Ed25519, "derived@test", passphrase).expect("key");
            let private = PrivateKey::from_openssh(text.as_bytes()).expect("parsed");
            let expected = private.public_key().to_openssh().expect("public");
            let mut data = Map::new();
            data.insert("private_key".into(), Value::String(text.to_string()));
            assert_eq!(public_key_line(&data).as_deref(), Some(expected.as_str()));
            assert!(expected.starts_with("ssh-ed25519 "));
            // The comment of an encrypted key is inside its encrypted part.
            assert_eq!(expected.ends_with(" derived@test"), passphrase.is_none());
            // A stored line wins.
            data.insert(
                "public_key".into(),
                Value::String("ssh-ed25519 AAAA x".into()),
            );
            assert_eq!(
                public_key_line(&data).as_deref(),
                Some("ssh-ed25519 AAAA x")
            );
        }
        assert_eq!(public_key_line(&Map::new()), None);
        let mut broken = Map::new();
        broken.insert("private_key".into(), Value::String("not a key".into()));
        assert_eq!(public_key_line(&broken), None);
    }
}
