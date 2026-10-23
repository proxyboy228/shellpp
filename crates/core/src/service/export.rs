//! Export and import of all data in the native JSON format (SPEC,
//! principle 4; docs/storage-format.md, sections 13.1–13.3; night decision
//! D34): the plain export `shellpp.export.v1` and the encrypted
//! `shellpp.export.encrypted.v1`.
//!
//! Export: every live record of the personal vault except the recovery key
//! (section 13.2), sorted by record id, plus the app settings (`settings`,
//! an optional member that v1 readers ignore). The command history and the
//! connection log only when asked for (D50). The plain export holds the
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
//! - Groups, hosts, keys, identities, known hosts, snippets, snippet
//!   packages, forwarding rules, highlight rules and workspaces are
//!   imported (a workspace's host links are references like any other,
//!   D84), each checked by the same
//!   rules as a save (D63: a rule never starts on launch, and one that
//!   listens beyond loopback not on connect either, except in a full
//!   restore); the command history and the
//!   connection log too unless the user turns them off (D51), each history
//!   entry through the secret detector of live recording; device records
//!   are skipped and counted.
//! - A key this client cannot read is kept as it is, as "unknown" (D50), so
//!   nothing of the file is lost.
//! - Existing records are never overwritten (D21); a copy carries the
//!   `import_key` `json:<exported id>`, so the next import finds it.
//! - Only the settings of how the app looks and behaves are applied
//!   ([`imported_setting`], D41); commands, programs, URLs and security
//!   settings stay as they are here, except in a full restore (D50).
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
    DataExport, DataExportInput, DataImportContents, DataImportContentsInput, DataImportInput,
    DataImportRefusal, DataImportSummary, SettingsSetInput,
};
use super::records::{Plan, parts, vault_view};
use crate::crypto::{
    self, EXPORT_NONCE_LEN, EXPORT_SALT_LEN, KdfParams, PlatformClass, SealedExport,
};
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::history;
use crate::ids::{EXPORT_FORMAT_ENCRYPTED_V1, EXPORT_FORMAT_V1};
use crate::keys;
use crate::model::group::{self, GroupRules};
use crate::model::host::{self, HostRules};
use crate::model::identity::{self, PasswordChange};
use crate::model::known_host;
use crate::model::snippet;
use crate::model::time::{format_timestamp, is_timestamp};
use crate::model::types::RecordKind;
use crate::model::wipe::WipeOnDrop;
use crate::model::{connection_log, history_entry};
use crate::model::{key, text};

/// The largest import file accepted (a plain export, or the encrypted file).
pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
/// Most records one import takes.
pub const MAX_IMPORT_RECORDS: usize = 500_000;
/// Most JSON values one import file may hold, counted before it is parsed
/// (D171): as a tree, a value costs far more memory than its text, so a
/// file of tiny values (`[0,0,…]`) is refused before it is built. A real
/// export spends well over 8 bytes of text per value.
pub const MAX_IMPORT_VALUES: usize = 8_000_000;
/// The `import_key` prefix of an imported copy: `json:<exported id>`
/// (docs/data-model.md, section 18.4).
const JSON_IMPORT_KEY: &str = "json:";
/// The suffix of an imported copy whose id exists here with other content.
pub const CONFLICT_SUFFIX: &str = " (imported)";
/// Groups deeper than this, or in a cycle, go to the top level.
const MAX_GROUP_DEPTH: usize = 32;

/// The record types an import takes, in the order they are written: the
/// snippets after their packages, the workspaces, the history and the
/// connection log after the hosts they name (D51, D63, D84).
const IMPORTED: [RecordKind; 13] = [
    RecordKind::Group,
    RecordKind::Key,
    RecordKind::Identity,
    RecordKind::Host,
    RecordKind::KnownHost,
    RecordKind::SnippetPackage,
    RecordKind::Snippet,
    RecordKind::ForwardingRule,
    RecordKind::HighlightRule,
    RecordKind::Workspace,
    RecordKind::HistoryEntry,
    RecordKind::HistoryHintState,
    RecordKind::ConnectionLogEntry,
];
/// The largest key record kept as "unknown" (D50): a private key file is at
/// most 64 KiB (D35), with room for the other members.
const MAX_UNKNOWN_KEY_BYTES: usize = 256 * 1024;

/// The settings an import applies (D41): how the app and the terminal look
/// and behave. A setting that runs or opens something (the OS detection
/// script and its switch, the local shell, the SFTP open program), points
/// somewhere (the update manifest) or guards the user (auto-lock, the
/// paste, delete and name-conflict questions, what is recorded or logged)
/// never comes from a file: a shared "host list" could otherwise run its
/// script on every server the user opens (security.md, TB7). Nor does one
/// that belongs to the device it was set on ([`device_setting`]).
pub fn imported_setting(key: &str) -> bool {
    const TERMINAL_AND_SFTP: [&str; 17] = [
        "terminal.autocomplete",
        "terminal.autoreconnect",
        "terminal.bell_sound",
        "terminal.bold_is_bright",
        "terminal.copy_on_select",
        "terminal.cursor_blink",
        "terminal.cursor_style",
        "terminal.default_theme",
        "terminal.font_family",
        "terminal.font_size",
        "terminal.highlighting_enabled",
        "terminal.keepalive_s",
        "terminal.line_height",
        "terminal.min_contrast",
        "terminal.term",
        "sftp.parallel_transfers",
        "sftp.show_hidden",
    ];
    ["appearance.", "view.", "shortcuts.", "hints."]
        .iter()
        .any(|prefix| key.starts_with(prefix))
        || TERMINAL_AND_SFTP.contains(&key)
}

/// The settings of this device only: the last serial Connect names a port
/// of this device (D95), and the time of the last update check is this
/// device's (D97). An export leaves them out and no import applies them, a
/// full restore included.
pub fn device_setting(key: &str) -> bool {
    key.starts_with("serial.") || key == "updates.last_check"
}

/// Whether an export writes records of `kind`: never the recovery key
/// (docs/storage-format.md, section 13.2); the history and the connection
/// log only when asked for (D50).
fn exported(kind: RecordKind, input: &DataExportInput) -> bool {
    match kind {
        RecordKind::RecoveryKey => false,
        RecordKind::HistoryEntry | RecordKind::HistoryHintState => input.include_history,
        RecordKind::ConnectionLogEntry => input.include_connection_log,
        _ => true,
    }
}

/// Whether an import takes records of `kind` (D51).
fn imported(kind: RecordKind, input: &DataImportInput) -> bool {
    match kind {
        RecordKind::HistoryEntry | RecordKind::HistoryHintState => input.import_history,
        RecordKind::ConnectionLogEntry => input.import_connection_log,
        _ => IMPORTED.contains(&kind),
    }
}

/// The label member of a record type (for the conflict suffix).
fn label_member(kind: RecordKind) -> Option<&'static str> {
    match kind {
        RecordKind::Group | RecordKind::SnippetPackage | RecordKind::Workspace => Some("name"),
        RecordKind::Host
        | RecordKind::Key
        | RecordKind::Identity
        | RecordKind::Snippet
        | RecordKind::ForwardingRule => Some("label"),
        _ => None,
    }
}

/// The autostart flags of a forwarding rule.
const AUTOSTART: [&str; 2] = ["autostart_on_connect", "autostart_on_launch"];

/// A forwarding rule from a file starts on its own only when the user
/// connects to its host, and only when it listens on loopback (D63, as the
/// `~/.ssh/config` import, D62 item 5; security.md 11.3): `autostart_on_launch`
/// goes off, and `autostart_on_connect` too for a rule that listens beyond
/// loopback, unless the user restores a backup of their own. Returns whether
/// a flag was turned off; such a rule is compared without its flags
/// ([`Incoming::ignored`]), so a rule that is the same here stays "already
/// exists" and the next import of the same file finds the copy again.
fn as_imported_rule(data: &mut Map<String, Value>) -> bool {
    let wide = data
        .get("bind_address")
        .and_then(Value::as_str)
        .is_some_and(|address| !address.trim().is_empty() && snippet::is_wide_bind(address));
    let flags: &[&str] = if wide {
        &AUTOSTART
    } else {
        &["autostart_on_launch"]
    };
    let mut changed = false;
    for flag in flags {
        if data.get(*flag) == Some(&Value::Bool(true)) {
            data.insert((*flag).into(), Value::Bool(false));
            changed = true;
        }
    }
    changed
}

/// Turns off Agent Forwarding of a host taken from a file (D172): while it
/// is on, the host can sign with the user's keys elsewhere (security.md,
/// RR19), so it is the user's own choice, never a file's, unless they
/// restore a backup of their own. Returns whether it was on. Applied after
/// the comparison with the records here, so a host that is the same here
/// stays "already here".
fn as_imported_host(data: &mut Map<String, Value>) -> bool {
    let Some(Value::Object(ssh)) = data.get_mut("ssh") else {
        return false;
    };
    if ssh.get("agent_forwarding") != Some(&Value::Bool(true)) {
        return false;
    }
    ssh.insert("agent_forwarding".into(), Value::Bool(false));
    true
}

/// Whether `local` holds what `incoming` holds once the ids of `map` are
/// rewritten, the members that change with the place in a list and the
/// `ignored` ones aside. Compares in place: no copy of the secrets is made.
fn same_content_except(
    local: &Map<String, Value>,
    incoming: &Map<String, Value>,
    map: &HashMap<Uuid, Uuid>,
    ignored: &[&str],
) -> bool {
    let kept = |name: &str| name != "order_key" && !ignored.contains(&name);
    let members = |data: &Map<String, Value>| data.keys().filter(|name| kept(name)).count();
    members(local) == members(incoming)
        && incoming.iter().all(|(name, value)| {
            !kept(name)
                || local
                    .get(name)
                    .is_some_and(|here| same_value(here, value, map))
        })
}

/// Whether `copy`, a copy an earlier import made of a record, still holds
/// what `incoming` holds: the same content apart from its import key and
/// the suffix on its label. A copy the user changed, or an export of a
/// newer version of the record, does not match.
fn same_copy(
    kind: RecordKind,
    copy: &Map<String, Value>,
    incoming: &Map<String, Value>,
    map: &HashMap<Uuid, Uuid>,
    also_ignored: &[&str],
) -> bool {
    let label = label_member(kind);
    let mut ignored = vec!["import_key"];
    ignored.extend(label);
    ignored.extend(also_ignored);
    same_content_except(copy, incoming, map, &ignored)
        && label.is_none_or(|member| {
            copy.get(member).and_then(Value::as_str)
                == incoming
                    .get(member)
                    .and_then(Value::as_str)
                    .map(suffixed)
                    .as_deref()
        })
}

fn same_value(here: &Value, incoming: &Value, map: &HashMap<Uuid, Uuid>) -> bool {
    match (here, incoming) {
        (Value::String(here), Value::String(text)) => {
            here == text
                || Uuid::parse_str(text)
                    .ok()
                    .and_then(|id| map.get(&id))
                    .is_some_and(|id| id.to_string() == *here)
        }
        (Value::Array(here), Value::Array(items)) => {
            here.len() == items.len()
                && here
                    .iter()
                    .zip(items)
                    .all(|(here, item)| same_value(here, item, map))
        }
        (Value::Object(here), Value::Object(members)) => {
            here.len() == members.len()
                && members.iter().all(|(name, value)| {
                    here.get(name)
                        .is_some_and(|local| same_value(local, value, map))
                })
        }
        _ => here == incoming,
    }
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
    /// A forwarding rule whose autostart this import turned off (D63).
    quieted: bool,
}

impl Incoming {
    /// The members left out when this record is compared with one here: the
    /// autostart flags of a quieted rule (the local rule keeps the user's own
    /// choice; an import never overwrites it).
    fn ignored(&self) -> &'static [&'static str] {
        if self.quieted { &AUTOSTART } else { &[] }
    }
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

/// What an incoming record becomes.
enum Checked {
    /// The record as a save would write it.
    Record(Map<String, Value>),
    /// A key this client cannot read, kept as it is (D50).
    UnknownKey(Map<String, Value>),
    /// A history entry live recording would not have written (D51).
    LeftOut,
}

/// Checks and normalizes the data of an incoming record by the rules of a
/// save; `Err` holds the field errors.
fn check(
    kind: RecordKind,
    record_id: Uuid,
    data: &Map<String, Value>,
) -> Result<Checked, Vec<FieldError>> {
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
        RecordKind::Key => match check_key(&input)? {
            Some(built) => built,
            None => return Ok(Checked::UnknownKey(data.clone())),
        },
        RecordKind::HistoryEntry => return check_history(&input),
        RecordKind::HistoryHintState => check_hint_state(&input)?,
        RecordKind::ConnectionLogEntry => check_connection_log(&input)?,
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
        // References are not checked here: one to a record that is neither
        // here nor in the file stays and reads as missing (D63).
        RecordKind::SnippetPackage => {
            input.remove("order_key");
            snippet::build_package(&input, None)?
        }
        RecordKind::Snippet => {
            input.remove("order_key");
            snippet::build_snippet(&input, None)?
        }
        RecordKind::ForwardingRule => snippet::build_rule(&input, None)?,
        RecordKind::HighlightRule => crate::model::highlight_rule::build(&input, None)?,
        // The tabs by the rules of a save (32 at most, the shape of
        // Restore, no quick-connect target); a host link to a host that is
        // neither here nor in the file stays, as every reference (D63),
        // and opening leaves that pane out (D81).
        RecordKind::Workspace => crate::model::workspace::build(&input, None)?,
        _ => return Err(vec![FieldError::new("type", ValidationReason::NotAllowed)]),
    };
    if let Some(Value::String(order_key)) = order_key
        && crate::model::order_key::is_valid(&order_key)
    {
        built.insert("order_key".into(), Value::String(order_key));
    }
    restore_managed(kind, data, &mut built);
    Ok(Checked::Record(built))
}

fn invalid(field: &str) -> Vec<FieldError> {
    vec![FieldError::new(field, ValidationReason::Invalid)]
}

/// A history entry by the rules of live recording (D51): left out when the
/// secret detector or a capture rule would not have recorded its command.
fn check_history(data: &Map<String, Value>) -> Result<Checked, Vec<FieldError>> {
    let Some(Value::String(command)) = data.get("command") else {
        return Err(vec![FieldError::new("command", ValidationReason::Required)]);
    };
    let executed_at = data
        .get("executed_at")
        .and_then(Value::as_str)
        .filter(|text| is_timestamp(text))
        .ok_or_else(|| invalid("executed_at"))?;
    let host = match data.get("host") {
        None | Some(Value::Null) => None,
        link => Some(history_entry::read_host_link(link).ok_or_else(|| invalid("host"))?),
    };
    let origin = match data.get("origin").and_then(Value::as_str) {
        None => "typed",
        Some(origin @ ("typed" | "imported")) => origin,
        Some(_) => return Err(invalid("origin")),
    };
    let Ok(command) = history::imported_command(command).map(Zeroizing::new) else {
        return Ok(Checked::LeftOut);
    };
    let mut built = history_entry::new_typed(host, &command, executed_at);
    built.insert("origin".into(), Value::String(origin.to_owned()));
    Ok(Checked::Record(built))
}

/// A host's history hint state (docs/data-model.md, section 13.2).
fn check_hint_state(data: &Map<String, Value>) -> Result<Map<String, Value>, Vec<FieldError>> {
    let host = history_entry::read_host_link(data.get("host")).ok_or_else(|| invalid("host"))?;
    let dismissed = data
        .get("dismissed")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("dismissed"))?;
    let mut built = Map::new();
    built.insert("host".into(), history_entry::host_link(host));
    built.insert("dismissed".into(), Value::Bool(dismissed));
    Ok(built)
}

/// A log entry as an import stores it, before it is compared with the one
/// here: without the members it never takes, and ended when still open
/// (see [`check_connection_log`]), so a second import finds it again.
fn as_imported_log_entry(data: &mut Map<String, Value>) {
    for member in ["recording_file_id", "published_copy"] {
        if let Some(mut value) = data.remove(member) {
            crate::model::wipe::wipe_value(&mut value);
        }
    }
    if connection_log::is_open(data) {
        connection_log::set_unknown_end(data);
    }
}

/// A connection log entry (docs/data-model.md, section 13.3): the members of
/// the section, each checked; a session recording (a file of the device
/// that wrote it) and a team copy are not taken. An entry still open in the
/// file ends as "unknown", as one left open by a crash.
fn check_connection_log(data: &Map<String, Value>) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut built = Map::new();
    let text = |name: &str, max: usize| -> Result<Option<String>, Vec<FieldError>> {
        match data.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(raw)) => text::string(raw, max).map_err(|_| invalid(name)),
            Some(_) => Err(invalid(name)),
        }
    };
    let time = |name: &str| -> Result<Option<String>, Vec<FieldError>> {
        match data.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(raw)) if is_timestamp(raw) => Ok(Some(raw.clone())),
            Some(_) => Err(invalid(name)),
        }
    };
    let id = |name: &str| -> Result<Option<Uuid>, Vec<FieldError>> {
        match data.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(raw)) => Uuid::parse_str(raw).map(Some).map_err(|_| invalid(name)),
            Some(_) => Err(invalid(name)),
        }
    };
    let started_at = time("started_at")?.ok_or_else(|| invalid("started_at"))?;
    built.insert("started_at".into(), Value::String(started_at));
    if let Some(ended_at) = time("ended_at")? {
        built.insert("ended_at".into(), Value::String(ended_at));
    }
    match data.get("end_reason") {
        None | Some(Value::Null) => {}
        Some(Value::String(reason))
            if matches!(
                reason.as_str(),
                "closed" | "exit_status" | "dropped" | crate::connlog::UNKNOWN_END
            ) =>
        {
            built.insert("end_reason".into(), Value::String(reason.clone()));
        }
        Some(_) => return Err(invalid("end_reason")),
    }
    match data.get("exit_status") {
        None | Some(Value::Null) => {}
        Some(Value::Number(status)) if status.is_i64() => {
            built.insert("exit_status".into(), Value::Number(status.clone()));
        }
        Some(_) => return Err(invalid("exit_status")),
    }
    match data.get("protocol").and_then(Value::as_str) {
        Some(protocol @ ("ssh" | "sftp" | "telnet" | "mosh")) => {
            built.insert("protocol".into(), Value::String(protocol.to_owned()));
        }
        _ => return Err(invalid("protocol")),
    }
    match data.get("host") {
        None | Some(Value::Null) => {}
        link => {
            let host = history_entry::read_host_link(link).ok_or_else(|| invalid("host"))?;
            built.insert("host".into(), history_entry::host_link(host));
        }
    }
    for (name, max) in [("host_label", 256), ("username", 256), ("user_email", 320)] {
        if let Some(value) = text(name, max)? {
            built.insert(name.into(), Value::String(value));
        }
    }
    let address = text("address", 255)?.ok_or_else(|| invalid("address"))?;
    built.insert("address".into(), Value::String(address));
    let port = data
        .get("port")
        .and_then(Value::as_u64)
        .filter(|port| (1..=65535).contains(port))
        .ok_or_else(|| invalid("port"))?;
    built.insert("port".into(), Value::from(port));
    if let Some(Value::Object(os)) = data.get("os") {
        let fits = os.len() <= 8
            && os.values().all(|value| {
                value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|text| text.chars().count() <= 64)
            });
        if !fits {
            return Err(invalid("os"));
        }
        built.insert("os".into(), Value::Object(os.clone()));
    }
    if let Some(user_id) = id("user_id")? {
        built.insert("user_id".into(), Value::String(user_id.to_string()));
    }
    let device_id = id("device_id")?.ok_or_else(|| invalid("device_id"))?;
    built.insert("device_id".into(), Value::String(device_id.to_string()));
    match data.get("saved") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(saved)) => {
            built.insert("saved".into(), Value::Bool(*saved));
        }
        Some(_) => return Err(invalid("saved")),
    }
    if connection_log::is_open(&built) {
        connection_log::set_unknown_end(&mut built);
    }
    Ok(built)
}

/// The members a save never takes from its input (`host::MANAGED`,
/// `identity::MANAGED`, a key's `import_key`) come from the export here: `import_key` (so a
/// later import of the original source still sees the record) and a
/// host's detected `os`. Each is checked first; a bad value is left out.
fn restore_managed(
    kind: RecordKind,
    incoming: &Map<String, Value>,
    built: &mut Map<String, Value>,
) {
    if matches!(
        kind,
        RecordKind::Host
            | RecordKind::Group
            | RecordKind::Identity
            | RecordKind::Key
            | RecordKind::SnippetPackage
            | RecordKind::Snippet
            | RecordKind::ForwardingRule
            | RecordKind::Workspace
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

/// Room a record's payload needs besides its data: the envelope members,
/// and a conflict suffix and import key the import may add.
const PAYLOAD_ROOM: usize = 1024;

/// Whether `data` is too large for one record: such a record is refused on
/// its own, as a save would refuse it, instead of failing the whole
/// import. Counted, not copied: the record may hold secrets.
fn too_large(data: &Map<String, Value>) -> bool {
    let mut counter = crate::model::payload::ByteCounter(0);
    serde_json::to_writer(&mut counter, data).is_err()
        || counter.0 + PAYLOAD_ROOM > crypto::MAX_PAYLOAD_LEN
}

/// A stored key: its private key parses (OpenSSH, the stored form), the
/// label is valid, a certificate fits the key. `None`: a key with a valid
/// label that this client cannot read (another source, a format of a later
/// version, a damaged key), kept as it is (D50).
fn check_key(data: &Map<String, Value>) -> Result<Option<Map<String, Value>>, Vec<FieldError>> {
    let label = match data.get("label").and_then(Value::as_str) {
        Some(raw) => match text::string(raw, key::MAX_LABEL_CHARS) {
            Ok(Some(label)) => label,
            Ok(None) => return Err(vec![FieldError::new("label", ValidationReason::Required)]),
            Err(reason) => return Err(vec![FieldError::new("label", reason)]),
        },
        None => return Err(vec![FieldError::new("label", ValidationReason::Required)]),
    };
    let unknown = || {
        // Counted, not copied: the record holds key material.
        let mut counter = crate::model::payload::ByteCounter(0);
        match serde_json::to_writer(&mut counter, data) {
            Ok(()) if counter.0 <= MAX_UNKNOWN_KEY_BYTES => Ok(None),
            _ => Err(vec![FieldError::new("record", ValidationReason::TooLong)]),
        }
    };
    // Agent keys (FIDO2 fallback, D5) arrive with a later stage.
    if key::source(data) != "stored" {
        return unknown();
    }
    let Some(private) = key::private_key(data) else {
        return unknown();
    };
    let Ok(parsed) = keys::parse(private) else {
        return unknown();
    };
    let mut built = key::new_stored(
        &label,
        &parsed.text,
        &parsed.public_key,
        key::passphrase(data).filter(|_| parsed.encrypted),
    );
    // A security key's `fido2` member (data-model 9.1, D37) comes from the
    // key itself, as on a Keychain import, never from the file.
    if let Some(fido2) = keys::fido2_member(&parsed.text) {
        built.insert("fido2".into(), fido2);
    }
    if let Some(certificate) = data.get("certificate").and_then(Value::as_str) {
        match crate::service::keychain::check_certificate(certificate, &built) {
            Ok(Some(line)) => {
                built.insert("certificate".into(), Value::String(line));
            }
            Ok(None) => {}
            Err(error) => return Err(vec![error]),
        }
    }
    Ok(Some(built))
}

impl Core {
    pub(super) fn data_export_impl(&self, input: DataExportInput) -> Result<DataExport, CoreError> {
        let password = input
            .password
            .clone()
            .filter(|password| !password.expose_secret().is_empty());
        if let Some(password) = &password
            && !crypto::password_length_ok(password)
        {
            return Err(refuse("password", ValidationReason::TooShort));
        }
        let now = self.now();
        let exported_at = format_timestamp(now);
        let (document, counts) = 'build: {
            let mut guard = self.inner();
            let settings = guard.settings.values();
            let mut parts = parts(&mut guard)?;
            for (class, wanted) in [
                (classes::HISTORY, input.include_history),
                (classes::CONNECTION_LOG, input.include_connection_log),
            ] {
                if wanted {
                    self.load_lazy(&mut parts, class, now)?;
                }
            }
            let vault = vault_view(parts.session);
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            if !input.recently_deleted.is_empty() {
                // Entries of Recently deleted (UI 14): their last live
                // versions, in the format of an ordinary export.
                let list = super::sync_ops::recently_deleted_export_list(
                    parts.store,
                    parts.session,
                    &input.recently_deleted,
                    &mut counts,
                )?;
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
                document.insert("settings".into(), Value::Object(Map::new()));
                break 'build (WipeOnDrop::new(document), counts);
            }
            let mut records: Vec<_> = parts
                .session
                .index
                .iter()
                .filter(|record| exported(record.kind, &input))
                .collect();
            records.sort_by_key(|record| record.record_id);
            let mut list = Vec::with_capacity(records.len());
            for record in records {
                *counts.entry(record.kind.name().to_owned()).or_default() += 1;
                // A workspace names the hosts the file carries only: never
                // one that is gone (D94).
                let data = match record.kind {
                    RecordKind::Workspace => {
                        crate::model::workspace::keep_host_names(record.data(), |link| {
                            parts
                                .session
                                .index
                                .get_kind(&link.record_id, RecordKind::Host)
                                .is_some()
                        })
                    }
                    _ => record.data().clone(),
                };
                list.push(json!({
                    "record_id": record.record_id.to_string(),
                    "revision": record.revision,
                    "v": record.payload.v,
                    "type": record.kind.name(),
                    "hlc": record.payload.hlc.to_text(),
                    "device_id": record.payload.device_id.to_string(),
                    "data": Value::Object(data),
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
                Value::Object(
                    settings
                        .into_iter()
                        .filter(|(key, _)| !device_setting(key))
                        .collect(),
                ),
            );
            (WipeOnDrop::new(document), counts)
        };
        // Two passes, as for payloads: the length first, then one buffer of
        // exactly that size (a growing one leaves partial copies of the
        // plaintext, secrets included, in the freed smaller buffers;
        // docs/security.md, section 9.5).
        let plain = {
            let mut counter = crate::model::payload::ByteCounter(0);
            serde_json::to_writer_pretty(&mut counter, &*document)
                .map_err(|_| CoreError::Internal("export serialization"))?;
            let mut bytes = Zeroizing::new(Vec::with_capacity(counter.0));
            serde_json::to_writer_pretty(&mut *bytes, &*document)
                .map_err(|_| CoreError::Internal("export serialization"))?;
            bytes
        };
        drop(document);
        let plain_text =
            std::str::from_utf8(&plain).map_err(|_| CoreError::Internal("export serialization"))?;
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
                text: SecretString::from(plain_text.to_owned()),
                encrypted: false,
                counts,
                file_name: format!("shellpp-export-{stamp}.json"),
            });
        };
        let params = KdfParams::preset(PlatformClass::Desktop);
        let sealed = crypto::seal_export(&password, &params, plain_text.as_bytes())?;
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
        if json_values(text) > MAX_IMPORT_VALUES {
            return Err(refuse("text", ValidationReason::TooMany));
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
                if json_values(text) > MAX_IMPORT_VALUES {
                    return Err(refuse("text", ValidationReason::TooMany));
                }
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
        // Every element of `records` counts, a refused or repeated one too:
        // the list of refusals stays bounded (D171).
        let mut examined = 0usize;
        // The exported personal vault: the history and the connection log
        // name its hosts by vault and record id (`host_link`).
        let mut exported_vaults: Vec<Uuid> = Vec::new();
        let vaults = document
            .get("vaults")
            .and_then(Value::as_array)
            .ok_or_else(|| refuse("text", ValidationReason::Invalid))?;
        for vault in vaults {
            let Some(records) = vault.get("records").and_then(Value::as_array) else {
                continue;
            };
            if let Some(vault_id) = vault
                .get("vault_id")
                .and_then(Value::as_str)
                .and_then(|raw| Uuid::parse_str(raw).ok())
            {
                exported_vaults.push(vault_id);
            }
            for record in records {
                if examined >= MAX_IMPORT_RECORDS {
                    return Err(refuse("text", ValidationReason::TooMany));
                }
                examined += 1;
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
                    (Some(kind), Some(record_id), Some(data)) if imported(kind, &input) => {
                        // Duplicates inside one file: the first one counts.
                        if seen.insert(record_id) {
                            let mut data = WipeOnDrop::new(data.clone());
                            if kind == RecordKind::ConnectionLogEntry {
                                as_imported_log_entry(&mut data);
                            }
                            let quieted = kind == RecordKind::ForwardingRule
                                && !input.full_restore
                                && as_imported_rule(&mut data);
                            incoming.push(Incoming {
                                kind,
                                record_id,
                                data,
                                quieted,
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
        let settings: Option<SettingsSetInput> = (input.apply_settings || input.full_restore)
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
            // The entries here, to compare the incoming ones with.
            for (class, wanted) in [
                (classes::HISTORY, input.import_history),
                (classes::CONNECTION_LOG, input.import_connection_log),
            ] {
                if wanted {
                    self.load_lazy(&mut parts, class, now)?;
                }
            }
            // References are rewritten to the record that stays (existing)
            // or to the imported copy (conflict), and a host link's vault to
            // the personal vault here.
            let mut map: HashMap<Uuid, Uuid> = HashMap::new();
            let personal = vault_view(parts.session).vault_id;
            for vault_id in exported_vaults {
                if vault_id != personal {
                    map.insert(vault_id, personal);
                }
            }
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
            // Copies made by earlier imports, by the id they were copied
            // from (`import_key` "json:<id>", data-model 18.4).
            // A record copied twice (two versions) has two copies.
            let mut copies: HashMap<(RecordKind, Uuid), Vec<Uuid>> = HashMap::new();
            for record in IMPORTED
                .iter()
                .flat_map(|kind| parts.session.index.of_kind(*kind))
            {
                let source = record
                    .data()
                    .get("import_key")
                    .and_then(Value::as_str)
                    .and_then(|key| key.strip_prefix(JSON_IMPORT_KEY))
                    .and_then(|source| Uuid::parse_str(source).ok());
                if let Some(source) = source {
                    copies
                        .entry((record.kind, source))
                        .or_default()
                        .push(record.record_id);
                }
            }
            // Every match is decided before any reference is rewritten.
            // First the keys and known hosts found here under another id:
            // they stay local, and a group or host that names one of them
            // points to the local record (export-format.md, section 4).
            let mut rest: Vec<Incoming> = Vec::new();
            for record in incoming {
                let local = parts.session.index.get(&record.record_id);
                if local.is_some_and(|local| {
                    local.kind == record.kind
                        && same_content_except(local.data(), &record.data, &map, record.ignored())
                }) {
                    summary.existing += 1;
                    continue;
                }
                let same = match record.kind {
                    RecordKind::Key => keys::record_fingerprint(&record.data)
                        .and_then(|print| fingerprints.get(&print).copied()),
                    RecordKind::KnownHost => known_host::read(&record.data).and_then(|entry| {
                        known
                            .iter()
                            .find(|(local, _)| known_host::same_natural_key(local, &entry))
                            .map(|(_, existing)| *existing)
                    }),
                    _ => None,
                };
                if let Some(existing) = same {
                    if existing != record.record_id {
                        map.insert(record.record_id, existing);
                    }
                    summary.existing += 1;
                } else {
                    rest.push(record);
                }
            }
            // Then the copies earlier imports made of these records, while
            // they still hold the same: until no other one is found, since a
            // copy that names another copied record matches only once that
            // one is mapped.
            loop {
                let index = &parts.session.index;
                let found: Vec<(usize, Uuid)> = rest
                    .iter()
                    .enumerate()
                    .filter_map(|(at, record)| {
                        let local = index.get(&record.record_id)?;
                        if local.kind == record.kind
                            && same_content_except(
                                local.data(),
                                &record.data,
                                &map,
                                record.ignored(),
                            )
                        {
                            return None;
                        }
                        copies
                            .get(&(record.kind, record.record_id))?
                            .iter()
                            .find(|copy| {
                                index.get_kind(copy, record.kind).is_some_and(|held| {
                                    same_copy(
                                        record.kind,
                                        held.data(),
                                        &record.data,
                                        &map,
                                        record.ignored(),
                                    )
                                })
                            })
                            .map(|copy| (at, *copy))
                    })
                    .collect();
                if found.is_empty() {
                    break;
                }
                for (at, copy) in found.into_iter().rev() {
                    let record = rest.remove(at);
                    map.insert(record.record_id, copy);
                    summary.existing += 1;
                }
            }
            // Then the others, compared with those references rewritten: an
            // id here with other content is a conflict.
            let mut decided: Vec<(Incoming, Option<Uuid>)> = Vec::new();
            for record in rest {
                let local = parts.session.index.get(&record.record_id);
                match local {
                    Some(local)
                        if local.kind == record.kind
                            && same_content_except(
                                local.data(),
                                &record.data,
                                &map,
                                record.ignored(),
                            ) =>
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
                    Ok(Checked::Record(data)) => data,
                    Ok(Checked::UnknownKey(data)) => {
                        summary.unknown_keys += 1;
                        data
                    }
                    Ok(Checked::LeftOut) => {
                        summary.history_left_out += 1;
                        continue;
                    }
                    Err(errors) => {
                        summary.refused.push(DataImportRefusal {
                            record_id: record.record_id,
                            record_type: record.kind.name().to_owned(),
                            errors,
                        });
                        continue;
                    }
                };
                if too_large(&data) {
                    summary.refused.push(DataImportRefusal {
                        record_id: record.record_id,
                        record_type: record.kind.name().to_owned(),
                        errors: vec![FieldError::new("record", ValidationReason::TooLong)],
                    });
                    continue;
                }
                // A host from a file never forwards the agent by itself
                // (D172), as a rule from a file never starts by itself (D63).
                if record.kind == RecordKind::Host
                    && !input.full_restore
                    && as_imported_host(&mut data)
                {
                    summary.hosts_agent_forwarding_off += 1;
                }
                if copy.is_some() {
                    if let Some(member) = label_member(record.kind)
                        && let Some(label) = data.get(member).and_then(Value::as_str)
                    {
                        let label = suffixed(label);
                        data.insert(member.into(), Value::String(label));
                    }
                    // A later import of the same record finds this copy.
                    if label_member(record.kind).is_some() {
                        data.insert(
                            "import_key".into(),
                            Value::String(format!("{JSON_IMPORT_KEY}{}", record.record_id)),
                        );
                    }
                    summary.conflicts += 1;
                } else {
                    summary.added += 1;
                }
                if record.quieted {
                    summary.rules_autostart_off += 1;
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
            let local = self.inner().settings.values();
            // Settings one by one: an unknown or invalid key is left out,
            // and so is every setting D41 keeps local (counted when the
            // file's value differs from the one here); those of the device
            // stay also in a full restore (D95).
            for (key, value) in settings.values {
                if device_setting(&key) || (!input.full_restore && !imported_setting(&key)) {
                    if local.get(&key).is_some_and(|here| *here != value) {
                        summary.settings_left_out += 1;
                    }
                    continue;
                }
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
            unknown_keys = summary.unknown_keys,
            history_left_out = summary.history_left_out,
            rules_autostart_off = summary.rules_autostart_off,
            hosts_agent_forwarding_off = summary.hosts_agent_forwarding_off,
            full_restore = input.full_restore,
            "data imported"
        );
        Ok(summary)
    }

    /// What a file holds that Import Data asks about (D51): its history and
    /// connection log entries. Slow for an encrypted file (Argon2id).
    pub(super) fn data_import_contents_impl(
        &self,
        input: DataImportContentsInput,
    ) -> Result<DataImportContents, CoreError> {
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
        let mut contents = DataImportContents::default();
        let records = document
            .get("vaults")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|vault| vault.get("records").and_then(Value::as_array))
            .flatten();
        for record in records {
            match record
                .get("type")
                .and_then(Value::as_str)
                .and_then(RecordKind::from_name)
            {
                Some(RecordKind::HistoryEntry) => contents.history += 1,
                Some(RecordKind::ConnectionLogEntry) => contents.connection_log += 1,
                _ => {}
            }
        }
        Ok(contents)
    }
}

/// At most how many JSON values `text` holds, counted without parsing it:
/// one, plus every `,`, `:`, `[` and `{` outside strings (D171).
fn json_values(text: &str) -> usize {
    let (mut count, mut in_string, mut escaped) = (1usize, false, false);
    for byte in text.bytes() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b',' | b':' | b'[' | b'{' => count += 1,
                _ => {}
            }
        }
    }
    count
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

/// Run 4, plan 9.7: the import checks of members a hostile or damaged file
/// can hold (coverage showed them untested).
#[cfg(test)]
mod check_tests {
    use serde_json::{Map, Value, json};

    use super::{check_connection_log, check_hint_state};

    const VAULT: &str = "0199f6a1-0000-7000-8000-0000000000a1";
    const HOST: &str = "0199f6a1-0000-7000-8000-0000000000b1";
    const DEVICE: &str = "0199f6a1-0000-7000-8000-0000000000c1";

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn entry() -> Value {
        json!({
            "started_at": "2026-10-22T01:02:03.000Z",
            "ended_at": "2026-10-22T01:05:00.000Z",
            "end_reason": "closed",
            "exit_status": 0,
            "protocol": "ssh",
            "host": {"vault_id": VAULT, "record_id": HOST},
            "host_label": "web",
            "address": "192.0.2.5",
            "port": 22,
            "device_id": DEVICE,
            "os": {"family": "linux", "name": null},
            "saved": true,
        })
    }

    #[test]
    fn imported_log_entries_are_checked_member_by_member() {
        let built = check_connection_log(&map(entry())).unwrap();
        assert_eq!(built["port"], json!(22));
        assert_eq!(built["saved"], json!(true));
        assert_eq!(built["end_reason"], json!("closed"));
        // An entry still open in the file ends with an unknown end.
        let mut open = map(entry());
        open.remove("ended_at");
        open.remove("end_reason");
        let built = check_connection_log(&open).unwrap();
        assert_eq!(built["end_reason"], json!(crate::connlog::UNKNOWN_END));
        // Each malformed member is refused by its name.
        for (member, bad) in [
            ("started_at", json!("yesterday")),
            ("ended_at", json!(5)),
            ("end_reason", json!("crashed")),
            ("exit_status", json!("0")),
            ("protocol", json!("rdp")),
            ("host", json!({"vault_id": VAULT})),
            ("username", json!(7)),
            ("address", Value::Null),
            ("port", json!(0)),
            ("port", json!(70000)),
            ("os", json!({"family": "x".repeat(65)})),
            ("user_id", json!("not-a-uuid")),
            ("device_id", Value::Null),
            ("saved", json!("yes")),
        ] {
            let mut data = map(entry());
            data.insert(member.into(), bad);
            let errors = check_connection_log(&data).expect_err(member);
            assert_eq!(errors[0].field, member, "{member}");
        }
    }

    #[test]
    fn imported_hint_states_need_a_host_and_a_flag() {
        let host = json!({"vault_id": VAULT, "record_id": HOST});
        let built = check_hint_state(&map(json!({"host": host, "dismissed": true}))).unwrap();
        assert_eq!(built["dismissed"], json!(true));
        let field = |value: Value| check_hint_state(&map(value)).unwrap_err()[0].field.clone();
        assert_eq!(field(json!({"host": host})), "dismissed");
        assert_eq!(field(json!({"host": host, "dismissed": "no"})), "dismissed");
        assert_eq!(field(json!({"dismissed": true})), "host");
    }
}
