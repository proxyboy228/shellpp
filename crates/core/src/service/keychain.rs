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
use super::dto::{CoreEventKind, KeyView, RecordBase, RecordRef};
use super::records::{check_vault, conflict_copy, deleted_version, parts};
use super::session::Session;
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::keys;
use crate::model::identity::{self, PasswordChange};
use crate::model::key;
use crate::model::partial;
use crate::model::payload::Payload;
use crate::model::text::{self, fold};
use crate::model::time::{format_unix_ms, uuid_v7_timestamp};
use crate::model::types::RecordKind;
use crate::model::wipe::{WipeOnDrop, wipe_value};
use crate::store::RecordRow;

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
    /// The stored revision: the `base` of a partial save (D221).
    pub revision: u64,
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
    /// The identity as the dialog opened it (its revision and its `label`,
    /// `username` and `key_id`): the save writes only what the edit changed
    /// (D221). `None` writes the input whole.
    pub base: Option<RecordBase>,
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

/// A record named in [`KeyUsers`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct KeyUser {
    pub record_id: Uuid,
    /// The host's label (else its address), the group's or the identity's
    /// name.
    pub name: String,
}

/// The records that use a key ([`Core::key_users`]): what the confirmation
/// of Remove lists (D50, item 2). Each list is sorted by name.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct KeyUsers {
    pub hosts: Vec<KeyUser>,
    pub groups: Vec<KeyUser>,
    pub identities: Vec<KeyUser>,
}

impl KeyUsers {
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty() && self.groups.is_empty() && self.identities.is_empty()
    }
}

/// The members of a host's or group's `ssh` that name a key (data-model
/// 7.3, 9.1).
const SSH_KEY_MEMBERS: [&str; 3] = ["key_id", "certificate_key_id", "fido2_key_id"];

fn names_key(member: Option<&Value>, key: Uuid) -> bool {
    member
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        == Some(key)
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
        revision: record.revision,
    })
}

/// An identity's data as the dialog sees it: without its password (wiped).
fn public_identity(mut data: Map<String, Value>) -> Map<String, Value> {
    for path in identity::SECRET_PATHS {
        if let Some(mut secret) = data.remove(*path) {
            wipe_value(&mut secret);
        }
    }
    data
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
    ///
    /// With `base` (the identity as the dialog opened it) the save is
    /// partial, as `record_save`'s (D221): only the members the edit changed
    /// are written onto the identity as it is now, in one transaction; a
    /// version it overwrites in a member it changed too, or whose password
    /// it replaces after the revision moved on, is kept as a conflict copy;
    /// an identity a pull deleted meanwhile comes back with the edit, the
    /// deletion kept as a copy. A save that changes nothing writes nothing.
    pub fn identity_save(&self, input: IdentitySaveInput) -> Result<IdentityView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        // The deletion a partial save undoes, with the last live version.
        let mut revived: Option<(RecordRow, Payload, Payload)> = None;
        let stored: Option<WipeOnDrop> = match input.record_id {
            Some(id) => match parts.session.index.get_kind(&id, RecordKind::Identity) {
                Some(record) => Some(WipeOnDrop::new(record.data().clone())),
                None if input.base.is_some() => {
                    let found = deleted_version(&mut parts, RecordKind::Identity, id)?;
                    let live = WipeOnDrop::new(found.2.data.clone().unwrap_or_default());
                    revived = Some(found);
                    Some(live)
                }
                None => return Err(CoreError::NotFound),
            },
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
        // A partial save checks the key only when the edit links another one
        // (a key deleted by a pull does not block the other members).
        let base_key = input
            .base
            .as_ref()
            .map(|base| base.data.get("key_id").and_then(Value::as_str))
            .map(|raw| raw.and_then(|raw| Uuid::parse_str(raw.trim()).ok()));
        if let Some(key_id) = input.key_id
            && base_key != Some(Some(key_id))
            && parts
                .session
                .index
                .get_kind(&key_id, RecordKind::Key)
                .is_none()
        {
            errors.push(FieldError::new("key_id", ValidationReason::MissingRef));
        }

        // A partial save (D221): the members the edit changed against its
        // base, written onto the identity as it is now.
        let mut same_members = false;
        if let (Some(base), Some(current), Some(id)) =
            (&input.base, stored.as_deref(), input.record_id)
        {
            // The edit and its base as the stored data would hold them, so
            // text in another normalization is no change.
            let edited = match identity::build(&data, Some(current), &PasswordChange::Keep) {
                Ok(built) => WipeOnDrop::new(public_identity(built)),
                Err(mut refused) => {
                    refused.extend(errors);
                    return Err(CoreError::Validation(refused));
                }
            };
            let base_data = match identity::build(&base.data, Some(current), &PasswordChange::Keep)
            {
                Ok(built) => WipeOnDrop::new(public_identity(built)),
                Err(_) => WipeOnDrop::new(base.data.clone()),
            };
            let current_public = WipeOnDrop::new(public_identity(current.clone()));
            let changes = partial::changes(&base_data, &edited, &[]);
            if changes.is_empty() && matches!(input.password, PasswordChange::Keep) {
                // Nothing changed: no new version, and nothing brought back.
                return match revived {
                    Some(_) => Err(CoreError::NotFound),
                    None => identity_view(parts.session, id),
                };
            }
            same_members = !partial::conflicts(&base_data, &current_public, &changes).is_empty();
            let mut merged = current_public.clone();
            partial::apply(&mut merged, &changes, &edited);
            data = merged;
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
        let built = WipeOnDrop::new(built);

        // The versions the save overwrites, kept as conflict copies (D221).
        let mut losing: Vec<(RecordRow, Payload)> = Vec::new();
        if let (Some(base), Some(current), Some(id)) =
            (&input.base, stored.as_deref(), input.record_id)
        {
            // The dialog never sees the password, so a password changed
            // since the base cannot be told apart: once the revision moved
            // on, a version whose password this save replaces or drops is
            // kept whole.
            let moved_on = revived.is_some()
                || parts
                    .session
                    .index
                    .get(&id)
                    .is_some_and(|record| record.revision != base.revision);
            let password_lost = moved_on
                && identity::password(current).is_some()
                && identity::password(current) != identity::password(&built);
            match &revived {
                Some((row, tombstone, live)) => {
                    losing.push((row.clone(), tombstone.clone()));
                    if same_members || password_lost {
                        losing.push((row.clone(), live.clone()));
                    }
                }
                None if same_members || password_lost => {
                    let record = parts
                        .session
                        .index
                        .get(&id)
                        .ok_or(CoreError::Internal("edited identity missing"))?;
                    let payload = record.payload.clone();
                    let row = parts
                        .store
                        .record(&parts.session.ring.vault_id(), &id)?
                        .ok_or(CoreError::Internal("edited identity row missing"))?;
                    losing.push((row, payload));
                }
                None => {}
            }
        }

        let record_id = input.record_id.unwrap_or_else(|| parts.ids.next(now));
        let mut copies = Vec::with_capacity(losing.len());
        for (row, payload) in &losing {
            copies.push(conflict_copy(parts.session, row, payload, now)?);
        }
        let write = parts.session.prepare_write(
            parts.store,
            RecordKind::Identity,
            record_id,
            built.into_inner(),
            now,
        )?;
        self.commit_records_then(
            &mut parts,
            vec![write],
            Vec::new(),
            Vec::new(),
            true,
            |tx| {
                for copy in &copies {
                    tx.record_put(copy)?;
                }
                Ok(())
            },
        )?;
        if !copies.is_empty() {
            tracing::info!(record_id = %record_id, "an identity edit kept the version it overwrote as a conflict copy");
            self.emit_with(parts.v, CoreEventKind::SyncChanged {});
        }
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

    /// What the key panel shows of a key's certificate (UI 7.3 block 5;
    /// stage 3): principals, the validity window, the CA fingerprint and the
    /// status at this moment; `None` when the key has no certificate.
    pub fn key_certificate_info(
        &self,
        key: RecordRef,
    ) -> Result<Option<keys::certificate::CertificateInfo>, CoreError> {
        let now = self
            .now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
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
            .and_then(|line| keys::certificate::certificate_info(line, now)))
    }

    /// The Certificate action (UI 7.1; docs/data-model.md, 9.2): stores the
    /// certificate (`*-cert.pub` text) on the key of `vault_id` whose public
    /// key it certifies. `certificate` `missing_ref` when no key matches (the
    /// private key would be needed), the errors of [`check_certificate`]
    /// otherwise.
    pub fn certificate_import(&self, vault_id: Uuid, text: &str) -> Result<KeyView, CoreError> {
        let line = text
            .trim()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let certificate = Certificate::from_openssh(&line).map_err(|_| {
            CoreError::Validation(vec![FieldError::new(
                "certificate",
                ValidationReason::Invalid,
            )])
        })?;
        let target = {
            let inner = self.inner();
            let session = Self::require_session(&inner)?;
            check_vault(session, Some(vault_id))?;
            let mut ids: Vec<Uuid> = session
                .index
                .of_kind(RecordKind::Key)
                .filter(|record| {
                    key_public_data(record.data()).as_ref() == Some(certificate.public_key())
                })
                .map(|record| record.record_id)
                .collect();
            ids.sort();
            ids.first().copied()
        };
        let Some(record_id) = target else {
            return Err(CoreError::Validation(vec![FieldError::new(
                "certificate",
                ValidationReason::MissingRef,
            )]));
        };
        self.key_certificate_set(KeyCertificateInput {
            key: RecordRef {
                vault_id,
                record_id,
            },
            certificate: line,
        })
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

    /// The hosts, groups and identities that use a key (its key, its
    /// certificate's key or its FIDO2 key; an identity's key): the
    /// confirmation of Remove lists them (D50). A removed key leaves them
    /// with a missing reference, which data-model 3.4 keeps as it is.
    /// `not_found` when the key is not there.
    pub fn key_users(&self, key: RecordRef) -> Result<KeyUsers, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(key.vault_id))?;
        session
            .index
            .get_kind(&key.record_id, RecordKind::Key)
            .ok_or(CoreError::NotFound)?;
        let id = key.record_id;
        let by_ssh = |kind: RecordKind| -> Vec<KeyUser> {
            session
                .index
                .of_kind(kind)
                .filter(|record| {
                    let ssh = record.data().get("ssh").and_then(Value::as_object);
                    SSH_KEY_MEMBERS
                        .iter()
                        .any(|member| names_key(ssh.and_then(|ssh| ssh.get(*member)), id))
                })
                .map(|record| {
                    let data = record.data();
                    let text = |name: &str| {
                        data.get(name)
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                    };
                    let name = match kind {
                        RecordKind::Group => text("name"),
                        _ => text("label").or_else(|| text("address")),
                    };
                    KeyUser {
                        record_id: record.record_id,
                        name: name.unwrap_or_default().to_owned(),
                    }
                })
                .collect()
        };
        let mut users = KeyUsers {
            hosts: by_ssh(RecordKind::Host),
            groups: by_ssh(RecordKind::Group),
            identities: session
                .index
                .of_kind(RecordKind::Identity)
                .filter(|record| names_key(record.data().get("key_id"), id))
                .map(|record| KeyUser {
                    record_id: record.record_id,
                    name: identity::label(record.data()).to_owned(),
                })
                .collect(),
        };
        for list in [&mut users.hosts, &mut users.groups, &mut users.identities] {
            list.sort_by_key(|user| fold(&user.name));
        }
        Ok(users)
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
