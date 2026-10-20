//! Export and import of all data in the native JSON format (SPEC,
//! principle 4; docs/storage-format.md, sections 13.1–13.3; night decision
//! D34): the plain export `shellpp.export.v1` and the encrypted
//! `shellpp.export.encrypted.v1`.
//!
//! Export: every live record of the personal vault except the recovery key
//! (section 13.2), sorted by record id, plus the app settings (`settings`,
//! an optional member that v1 readers ignore). The plain export holds the
//! secrets in clear text, the encrypted one only ciphertext.
//!
//! Import (D34): records are merged by record id into the personal vault.
//! - A new id is added under that id, so references between the imported
//!   records stay as they were.
//! - An id that exists here with the same content (the order key aside), a
//!   key with the same fingerprint or a known host with the same entry is
//!   "existing" and skipped; references to it point to the local record.
//! - An id that exists here with other content is a conflict: both versions
//!   are kept; the imported one gets a new id and an " (imported)" suffix
//!   on its label or name, and references inside the import point to it.
//! - Groups, hosts, keys, identities and known hosts are imported, each
//!   checked by the same rules as a save; history, the connection log and
//!   device records are skipped and counted.
//! - Existing records are never overwritten (D21).
//!
//! Nothing here logs record content; the plain text and its copies are
//! wiped.

use std::collections::{BTreeMap, HashMap, HashSet};

use data_encoding::BASE64URL_NOPAD;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use shellpp_sync_api::classes;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::Core;
use super::dto::{
    DataExport, DataExportInput, DataImportInput, DataImportRefusal, DataImportSummary,
    SettingsSetInput,
};
use super::records::{Plan, parts, vault_view};
use crate::crypto::{
    self, EXPORT_NONCE_LEN, EXPORT_SALT_LEN, KdfParams, PlatformClass, SealedExport,
};
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::ids::{EXPORT_FORMAT_ENCRYPTED_V1, EXPORT_FORMAT_V1};
use crate::keys;
use crate::model::group::{self, GroupRules};
use crate::model::host::{self, HostRules};
use crate::model::identity::{self, PasswordChange};
use crate::model::known_host;
use crate::model::time::format_timestamp;
use crate::model::types::RecordKind;
use crate::model::wipe::WipeOnDrop;
use crate::model::{key, text};

/// The largest import file accepted (a plain export, or the encrypted file).
pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
/// Most records one import takes.
pub const MAX_IMPORT_RECORDS: usize = 500_000;
/// The suffix of an imported copy whose id exists here with other content.
pub const CONFLICT_SUFFIX: &str = " (imported)";
/// Groups deeper than this, or in a cycle, go to the top level.
const MAX_GROUP_DEPTH: usize = 32;

/// The record types an import takes, in the order they are written.
const IMPORTED: [RecordKind; 5] = [
    RecordKind::Group,
    RecordKind::Key,
    RecordKind::Identity,
    RecordKind::Host,
    RecordKind::KnownHost,
];

/// Records never exported (docs/storage-format.md, section 13.2).
fn exported(kind: RecordKind) -> bool {
    !matches!(kind, RecordKind::RecoveryKey)
}

/// The label member of a record type (for the conflict suffix).
fn label_member(kind: RecordKind) -> Option<&'static str> {
    match kind {
        RecordKind::Group => Some("name"),
        RecordKind::Host | RecordKind::Key | RecordKind::Identity => Some("label"),
        _ => None,
    }
}

/// `data` without the members that change with the place in a list.
fn comparable(data: &Map<String, Value>) -> Map<String, Value> {
    let mut data = data.clone();
    data.remove("order_key");
    data
}

/// Rewrites every string in `value` that is an id of `map`.
fn rewrite_ids(value: &mut Value, map: &HashMap<Uuid, Uuid>) {
    match value {
        Value::String(text) => {
            if let Ok(id) = Uuid::parse_str(text)
                && let Some(new) = map.get(&id)
            {
                *text = new.to_string();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite_ids(item, map)),
        Value::Object(members) => members
            .values_mut()
            .for_each(|member| rewrite_ids(member, map)),
        _ => {}
    }
}

/// `label` with the conflict suffix, within the label limit.
fn suffixed(label: &str) -> String {
    let keep = key::MAX_LABEL_CHARS.saturating_sub(CONFLICT_SUFFIX.chars().count());
    let mut out: String = label.chars().take(keep).collect();
    out.push_str(CONFLICT_SUFFIX);
    out
}

fn refuse(field: &str, reason: ValidationReason) -> CoreError {
    CoreError::Validation(vec![FieldError::new(field, reason)])
}

/// One record read from the import file.
struct Incoming {
    kind: RecordKind,
    record_id: Uuid,
    data: WipeOnDrop,
}

/// Takes the string at `path` (dotted) out of `data`.
fn take_secret(data: &mut Map<String, Value>, path: &[&str]) -> Option<SecretString> {
    let (last, parents) = path.split_last()?;
    let mut node = data;
    for parent in parents {
        node = node.get_mut(*parent)?.as_object_mut()?;
    }
    match node.remove(*last) {
        Some(Value::String(mut secret)) => {
            let out = SecretString::from(secret.clone());
            zeroize::Zeroize::zeroize(&mut secret);
            Some(out)
        }
        Some(mut other) => {
            crate::model::wipe::wipe_value(&mut other);
            None
        }
        None => None,
    }
}

/// Checks and normalizes the data of an incoming record by the rules of a
/// save; `Err` holds the field errors.
fn check(
    kind: RecordKind,
    record_id: Uuid,
    data: &Map<String, Value>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let order_key = data.get("order_key").cloned();
    let mut input = WipeOnDrop::new(data.clone());
    let mut built = match kind {
        RecordKind::Group => {
            input.remove("order_key");
            group::build(
                &input,
                None,
                &BTreeMap::new(),
                &GroupRules {
                    record_id: Some(record_id),
                },
            )?
        }
        RecordKind::Host => {
            input.remove("order_key");
            let mut secrets = BTreeMap::new();
            for path in [
                ["ssh", "password"].as_slice(),
                &["ssh", "proxy", "password"],
            ] {
                if let Some(secret) = take_secret(&mut input, path) {
                    secrets.insert(path.join("."), Some(secret));
                }
            }
            host::build(
                &input,
                None,
                &secrets,
                &HostRules {
                    record_id: Some(record_id),
                    personal_vault: true,
                },
            )?
        }
        RecordKind::Identity => {
            let password = match take_secret(&mut input, &["password"]) {
                Some(password) => PasswordChange::Set(password),
                None => PasswordChange::Keep,
            };
            identity::build(&input, None, &password)?
        }
        RecordKind::Key => check_key(&input)?,
        RecordKind::KnownHost => {
            let entry = known_host::read(&input)
                .ok_or_else(|| vec![FieldError::new("host", ValidationReason::Invalid)])?;
            let checked = known_host::validate(known_host::EntryInput {
                host: &entry.host,
                port: entry.port,
                key_type: &entry.key_type,
                public_key: &entry.public_key,
                revoked: entry.revoked,
                source: entry.source,
                comment: entry.comment.as_deref(),
            })?;
            known_host::to_data(&checked)
        }
        _ => return Err(vec![FieldError::new("type", ValidationReason::NotAllowed)]),
    };
    if let Some(Value::String(order_key)) = order_key
        && crate::model::order_key::is_valid(&order_key)
    {
        built.insert("order_key".into(), Value::String(order_key));
    }
    restore_managed(kind, data, &mut built);
    Ok(built)
}

/// The members a save never takes from its input (`host::MANAGED`,
/// `identity::MANAGED`) come from the export here: `import_key` (so a
/// later import of the original source still sees the record) and a
/// host's detected `os`. Each is checked first; a bad value is left out.
fn restore_managed(
    kind: RecordKind,
    incoming: &Map<String, Value>,
    built: &mut Map<String, Value>,
) {
    if matches!(
        kind,
        RecordKind::Host | RecordKind::Group | RecordKind::Identity
    ) && let Some(raw) = incoming.get("import_key").and_then(Value::as_str)
        && let Ok(Some(import_key)) = text::string(raw, MAX_IMPORT_KEY_CHARS)
    {
        built.insert("import_key".into(), Value::String(import_key));
    }
    if kind == RecordKind::Host
        && let Some(Value::Object(os)) = incoming.get("os")
        && os.values().all(|value| {
            value
                .as_str()
                .is_some_and(|text| text.chars().count() <= 64)
                || value.is_null()
        })
        && os.len() <= 8
    {
        built.insert("os".into(), Value::Object(os.clone()));
    }
}

/// `import_key` is a string(512) (docs/data-model.md, section 18.4).
const MAX_IMPORT_KEY_CHARS: usize = 512;

/// A stored key: its private key parses (OpenSSH, the stored form), the
/// label is valid, a certificate fits the key.
fn check_key(data: &Map<String, Value>) -> Result<Map<String, Value>, Vec<FieldError>> {
    let label = match data.get("label").and_then(Value::as_str) {
        Some(raw) => match text::string(raw, key::MAX_LABEL_CHARS) {
            Ok(Some(label)) => label,
            Ok(None) => return Err(vec![FieldError::new("label", ValidationReason::Required)]),
            Err(reason) => return Err(vec![FieldError::new("label", reason)]),
        },
        None => return Err(vec![FieldError::new("label", ValidationReason::Required)]),
    };
    if key::source(data) != "stored" {
        // Agent keys (FIDO2 fallback, D5) arrive with a later stage.
        return Err(vec![FieldError::new(
            "source",
            ValidationReason::NotAllowed,
        )]);
    }
    let private = key::private_key(data)
        .ok_or_else(|| vec![FieldError::new("private_key", ValidationReason::Required)])?;
    let parsed = keys::parse(private).map_err(|error| vec![error])?;
    let mut built = key::new_stored(
        &label,
        &parsed.text,
        &parsed.public_key,
        key::passphrase(data).filter(|_| parsed.encrypted),
    );
    if let Some(certificate) = data.get("certificate").and_then(Value::as_str) {
        match crate::service::keychain::check_certificate(certificate, &built) {
            Ok(Some(line)) => {
                built.insert("certificate".into(), Value::String(line));
            }
            Ok(None) => {}
            Err(error) => return Err(vec![error]),
        }
    }
    Ok(built)
}

impl Core {
    pub(super) fn data_export_impl(&self, input: DataExportInput) -> Result<DataExport, CoreError> {
        let password = input
            .password
            .filter(|password| !password.expose_secret().is_empty());
        if let Some(password) = &password
            && !crypto::password_length_ok(password)
        {
            return Err(refuse("password", ValidationReason::TooShort));
        }
        let now = self.now();
        let exported_at = format_timestamp(now);
        let (document, counts) = {
            let mut guard = self.inner();
            let settings = guard.settings.values();
            let mut parts = parts(&mut guard)?;
            for class in [classes::HISTORY, classes::CONNECTION_LOG] {
                self.load_lazy(&mut parts, class, now)?;
            }
            let vault = vault_view(parts.session);
            let mut records: Vec<_> = parts
                .session
                .index
                .iter()
                .filter(|record| exported(record.kind))
                .collect();
            records.sort_by_key(|record| record.record_id);
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            let mut list = Vec::with_capacity(records.len());
            for record in records {
                *counts.entry(record.kind.name().to_owned()).or_default() += 1;
                list.push(json!({
                    "record_id": record.record_id.to_string(),
                    "revision": record.revision,
                    "v": record.payload.v,
                    "type": record.kind.name(),
                    "hlc": record.payload.hlc.to_text(),
                    "device_id": record.payload.device_id.to_string(),
                    "data": Value::Object(record.data().clone()),
                }));
            }
            let mut document = Map::new();
            document.insert("format".into(), json!(EXPORT_FORMAT_V1));
            document.insert("exported_at".into(), json!(exported_at));
            document.insert("app_version".into(), json!(self.config.app_version));
            document.insert(
                "vaults".into(),
                json!([{
                    "vault_id": vault.vault_id.to_string(),
                    "kind": "personal",
                    "name": vault.name,
                    "records": list,
                }]),
            );
            document.insert(
                "settings".into(),
                Value::Object(settings.into_iter().collect()),
            );
            (WipeOnDrop::new(document), counts)
        };
        let plain = Zeroizing::new(
            serde_json::to_string_pretty(&*document)
                .map_err(|_| CoreError::Internal("export serialization"))?,
        );
        drop(document);
        let stamp = exported_at
            .replace(['-', ':'], "")
            .split('.')
            .next()
            .unwrap_or_default()
            .to_owned();
        let stamp = if stamp.ends_with('Z') {
            stamp
        } else {
            format!("{stamp}Z")
        };
        let Some(password) = password else {
            tracing::info!(records = counts.values().sum::<usize>(), "plain export");
            return Ok(DataExport {
                text: SecretString::from(plain.as_str().to_owned()),
                encrypted: false,
                counts,
                file_name: format!("shellpp-export-{stamp}.json"),
            });
        };
        let params = KdfParams::preset(PlatformClass::Desktop);
        let sealed = crypto::seal_export(&password, &params, plain.as_bytes())?;
        let file = json!({
            "format": EXPORT_FORMAT_ENCRYPTED_V1,
            "kdf": {"alg": params.alg, "v": params.v, "m_kib": params.m_kib, "t": params.t, "p": params.p},
            "salt": BASE64URL_NOPAD.encode(&sealed.salt),
            "nonce": BASE64URL_NOPAD.encode(&sealed.nonce),
            "ciphertext": BASE64URL_NOPAD.encode(&sealed.ciphertext),
        });
        let text = serde_json::to_string_pretty(&file)
            .map_err(|_| CoreError::Internal("export serialization"))?;
        tracing::info!(records = counts.values().sum::<usize>(), "encrypted export");
        Ok(DataExport {
            text: SecretString::from(text),
            encrypted: true,
            counts,
            file_name: format!("shellpp-export-{stamp}.encrypted.json"),
        })
    }

    /// The plain export inside `text`: as it is, or decrypted with
    /// `password`. Slow for an encrypted file (Argon2id): no mutex held.
    fn open_import(
        text: &str,
        password: Option<&SecretString>,
    ) -> Result<Zeroizing<String>, CoreError> {
        if text.len() > MAX_IMPORT_BYTES {
            return Err(refuse("text", ValidationReason::TooLong));
        }
        let outer: WipeOnDrop = match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => WipeOnDrop::new(map),
            _ => return Err(refuse("text", ValidationReason::Invalid)),
        };
        match outer.get("format").and_then(Value::as_str) {
            Some(EXPORT_FORMAT_V1) => Ok(Zeroizing::new(text.to_owned())),
            Some(EXPORT_FORMAT_ENCRYPTED_V1) => {
                let password = password
                    .filter(|password| !password.expose_secret().is_empty())
                    .ok_or_else(|| refuse("password", ValidationReason::Required))?;
                let kdf: KdfParams = outer
                    .get("kdf")
                    .cloned()
                    .and_then(|kdf| serde_json::from_value(kdf).ok())
                    .ok_or_else(|| refuse("text", ValidationReason::Invalid))?;
                let bytes = |name: &str| {
                    outer
                        .get(name)
                        .and_then(Value::as_str)
                        .and_then(|value| BASE64URL_NOPAD.decode(value.as_bytes()).ok())
                        .ok_or_else(|| refuse("text", ValidationReason::Invalid))
                };
                let salt: [u8; EXPORT_SALT_LEN] = bytes("salt")?
                    .try_into()
                    .map_err(|_| refuse("text", ValidationReason::Invalid))?;
                let nonce: [u8; EXPORT_NONCE_LEN] = bytes("nonce")?
                    .try_into()
                    .map_err(|_| refuse("text", ValidationReason::Invalid))?;
                let sealed = SealedExport {
                    salt,
                    nonce,
                    ciphertext: bytes("ciphertext")?,
                };
                let plain =
                    crypto::open_export(password, &kdf, &sealed).map_err(|error| match error {
                        crypto::CryptoError::Aead => refuse("password", ValidationReason::Invalid),
                        _ => refuse("text", ValidationReason::NotAllowed),
                    })?;
                let text = std::str::from_utf8(&plain)
                    .map_err(|_| refuse("text", ValidationReason::Invalid))?;
                Ok(Zeroizing::new(text.to_owned()))
            }
            _ => Err(refuse("text", ValidationReason::Invalid)),
        }
    }

    pub(super) fn data_import_impl(
        &self,
        input: DataImportInput,
    ) -> Result<DataImportSummary, CoreError> {
        {
            let inner = self.inner();
            Self::require_session(&inner)?;
        }
        let plain = Self::open_import(input.text.expose_secret(), input.password.as_ref())?;
        let document: WipeOnDrop = match serde_json::from_str::<Value>(&plain) {
            Ok(Value::Object(map)) => WipeOnDrop::new(map),
            _ => return Err(refuse("text", ValidationReason::Invalid)),
        };
        drop(plain);
        if document.get("format").and_then(Value::as_str) != Some(EXPORT_FORMAT_V1) {
            return Err(refuse("text", ValidationReason::Invalid));
        }
        let mut summary = DataImportSummary::default();
        let mut incoming: Vec<Incoming> = Vec::new();
        let mut seen: HashSet<Uuid> = HashSet::new();
        let vaults = document
            .get("vaults")
            .and_then(Value::as_array)
            .ok_or_else(|| refuse("text", ValidationReason::Invalid))?;
        for vault in vaults {
            let Some(records) = vault.get("records").and_then(Value::as_array) else {
                continue;
            };
            for record in records {
                if incoming.len() + summary.skipped >= MAX_IMPORT_RECORDS {
                    return Err(refuse("text", ValidationReason::TooMany));
                }
                let kind = record
                    .get("type")
                    .and_then(Value::as_str)
                    .and_then(RecordKind::from_name);
                let record_id = record
                    .get("record_id")
                    .and_then(Value::as_str)
                    .and_then(|raw| Uuid::parse_str(raw).ok());
                let data = record.get("data").and_then(Value::as_object);
                match (kind, record_id, data) {
                    (Some(kind), Some(record_id), Some(data)) if IMPORTED.contains(&kind) => {
                        // Duplicates inside one file: the first one counts.
                        if seen.insert(record_id) {
                            incoming.push(Incoming {
                                kind,
                                record_id,
                                data: WipeOnDrop::new(data.clone()),
                            });
                        }
                    }
                    (_, Some(_), Some(_)) => summary.skipped += 1,
                    (_, record_id, _) => summary.refused.push(DataImportRefusal {
                        record_id: record_id.unwrap_or_default(),
                        record_type: record
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .chars()
                            .take(64)
                            .collect(),
                        errors: vec![FieldError::new("record", ValidationReason::Invalid)],
                    }),
                }
            }
        }
        let settings: Option<SettingsSetInput> = input
            .apply_settings
            .then(|| document.get("settings").and_then(Value::as_object).cloned())
            .flatten()
            .map(|values| SettingsSetInput {
                values: values.into_iter().collect(),
                reset: Vec::new(),
            });
        drop(document);
        incoming.sort_by_key(|record| {
            IMPORTED
                .iter()
                .position(|kind| *kind == record.kind)
                .unwrap_or(usize::MAX)
        });

        let now = self.now();
        {
            let mut guard = self.inner();
            let mut parts = parts(&mut guard)?;
            // References are rewritten to the record that stays (existing)
            // or to the imported copy (conflict).
            let mut map: HashMap<Uuid, Uuid> = HashMap::new();
            let mut writes: Vec<(RecordKind, Uuid, Map<String, Value>)> = Vec::new();
            let fingerprints: HashMap<String, Uuid> = parts
                .session
                .index
                .of_kind(RecordKind::Key)
                .filter_map(|record| {
                    keys::record_fingerprint(record.data()).map(|print| (print, record.record_id))
                })
                .collect();
            let known: Vec<(known_host::Entry, Uuid)> = parts
                .session
                .index
                .of_kind(RecordKind::KnownHost)
                .filter_map(|record| {
                    known_host::read(record.data()).map(|entry| (entry, record.record_id))
                })
                .collect();
            let mut decided: Vec<(Incoming, Option<Uuid>)> = Vec::new();
            for record in incoming {
                let local = parts.session.index.get(&record.record_id);
                match local {
                    Some(local)
                        if local.kind == record.kind
                            && comparable(local.data()) == comparable(&record.data) =>
                    {
                        summary.existing += 1;
                    }
                    Some(_) => {
                        let copy = parts.ids.next(now);
                        map.insert(record.record_id, copy);
                        decided.push((record, Some(copy)));
                    }
                    None => decided.push((record, None)),
                }
            }
            for (mut record, copy) in decided {
                let mut value = Value::Object(std::mem::take(&mut *record.data));
                rewrite_ids(&mut value, &map);
                if let Value::Object(data) = value {
                    *record.data = data;
                }
                let target = copy.unwrap_or(record.record_id);
                let mut data = match check(record.kind, target, &record.data) {
                    Ok(data) => data,
                    Err(errors) => {
                        summary.refused.push(DataImportRefusal {
                            record_id: record.record_id,
                            record_type: record.kind.name().to_owned(),
                            errors,
                        });
                        continue;
                    }
                };
                // The same key or known host under another id stays local.
                if record.kind == RecordKind::Key
                    && let Some(existing) = keys::record_fingerprint(&data)
                        .and_then(|print| fingerprints.get(&print).copied())
                {
                    map.insert(record.record_id, existing);
                    summary.existing += 1;
                    crate::model::wipe::wipe_map(&mut data);
                    continue;
                }
                if record.kind == RecordKind::KnownHost
                    && let Some(entry) = known_host::read(&data)
                    && let Some((_, existing)) = known
                        .iter()
                        .find(|(local, _)| known_host::same_natural_key(local, &entry))
                {
                    map.insert(record.record_id, *existing);
                    summary.existing += 1;
                    continue;
                }
                if copy.is_some() {
                    if let Some(member) = label_member(record.kind)
                        && let Some(label) = data.get(member).and_then(Value::as_str)
                    {
                        let label = suffixed(label);
                        data.insert(member.into(), Value::String(label));
                    }
                    summary.conflicts += 1;
                } else {
                    summary.added += 1;
                }
                writes.push((record.kind, target, data));
            }
            // References to records that are neither here nor imported are
            // dropped by the rules of each type when they are read; group
            // parents that would form a cycle go to the top level.
            fix_group_parents(&mut writes, &parts.session.index);
            let mut plan = Plan::default();
            for (kind, record_id, data) in writes {
                plan.set(kind, record_id, data);
            }
            let prepared = plan.prepare(&mut parts, now)?;
            if !prepared.is_empty() {
                self.commit_records(&mut parts, prepared, Vec::new(), Vec::new())?;
            }
        }
        if let Some(settings) = settings {
            // Settings one by one: an unknown or invalid key is left out.
            for (key, value) in settings.values {
                let one = SettingsSetInput {
                    values: BTreeMap::from([(key, value)]),
                    reset: Vec::new(),
                };
                if self.settings_set(one).is_ok() {
                    summary.settings += 1;
                }
            }
        }
        tracing::info!(
            added = summary.added,
            existing = summary.existing,
            conflicts = summary.conflicts,
            skipped = summary.skipped,
            refused = summary.refused.len(),
            "data imported"
        );
        Ok(summary)
    }
}

/// Clears the `parent_id` of imported groups whose parent chain loops or
/// runs deeper than [`MAX_GROUP_DEPTH`] (the local tree included).
fn fix_group_parents(
    writes: &mut [(RecordKind, Uuid, Map<String, Value>)],
    index: &crate::index::Index,
) {
    let mut parents: HashMap<Uuid, Uuid> = index
        .of_kind(RecordKind::Group)
        .filter_map(|record| {
            record
                .data()
                .get("parent_id")
                .and_then(Value::as_str)
                .and_then(|raw| Uuid::parse_str(raw).ok())
                .map(|parent| (record.record_id, parent))
        })
        .collect();
    for (kind, id, data) in writes.iter() {
        if *kind != RecordKind::Group {
            continue;
        }
        match data
            .get("parent_id")
            .and_then(Value::as_str)
            .and_then(|raw| Uuid::parse_str(raw).ok())
        {
            Some(parent) => {
                parents.insert(*id, parent);
            }
            None => {
                parents.remove(id);
            }
        }
    }
    for (kind, id, data) in writes.iter_mut() {
        if *kind != RecordKind::Group {
            continue;
        }
        let mut node = *id;
        let mut bad = false;
        for _ in 0..=MAX_GROUP_DEPTH {
            match parents.get(&node) {
                Some(parent) if parent == id => {
                    bad = true;
                    break;
                }
                Some(parent) => node = *parent,
                None => break,
            }
        }
        if bad || parents.contains_key(&node) {
            data.remove("parent_id");
            parents.remove(id);
        }
    }
}
