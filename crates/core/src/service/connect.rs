//! Stage 2 facade bodies: session configuration, known hosts, keys, prompted
//! credentials, OS detection results, history and the minimal connection log
//! (stage 2 plan, section 4.4).
//!
//! Every write goes through the record path of `records.rs` (seal, one
//! transaction, index, `records:changed` with the record's class). Slow key
//! decryption (bcrypt-pbkdf) runs without the core mutex; the state is
//! checked again after re-acquiring it. Nothing here logs addresses, user
//! names, commands, keys or other record data.
//!
//! Owner: S4. The public signatures are in `service/mod.rs` (lead contract).

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use shellpp_sync_api::classes;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::Core;
use super::Inner;
use super::dto::{
    Backspace, ConnectionLogView, HistoryClearInput, HistoryClearResult, HistoryEntryView,
    HistoryListInput, HostOs, KeyImportInput, KeyImportResult, KeyView, KnownHostAddInput,
    KnownHostSource, KnownHostView, OsSource, RecentConnection, RecordRef, RecordsRef,
    SessionTarget,
};
use super::host_api::{
    ConnectionEndReason, ConnectionLogStart, HistoryDecision, HistoryDropReason,
    HistoryRecordInput, HostKeyCheck, HostKeyStatus, ProxyType, RouteProblem, SessionConfig,
    SessionKey, SessionProxy, StartupSnippet,
};
use super::records::{self, Parts, check_vault, parts};
use super::session::Session;
use crate::connlog::{PendingEnd, RECENT_LIMIT};
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::history;
use crate::index::{Index, IndexedRecord};
use crate::keys;
use crate::known_hosts;
use crate::model::connection_log;
use crate::model::fields::{self, BUILTIN_THEMES};
use crate::model::group;
use crate::model::history_entry;
use crate::model::host;
use crate::model::identity;
use crate::model::key;
use crate::model::known_host::{self, Entry, EntryInput};
use crate::model::text::{self, fold};
use crate::model::time::{format_timestamp, format_unix_ms, uuid_v7_timestamp};
use crate::model::types::RecordKind;
use crate::model::wipe::WipeOnDrop;
use crate::os_detect::{self, DetectedOs};
use crate::settings::Settings;

/// The default Telnet port (docs/data-model.md, section 7.4).
const TELNET_PORT: u16 = 23;

/// One row of Recent connections per host link, else per address and port.
type RecentKey = (Option<(Uuid, Uuid)>, String, u16);

/// Default and largest `history_list` limits.
const HISTORY_LIST_DEFAULT: u32 = 500;
const HISTORY_LIST_MAX: u32 = 5000;

/// Largest prompted password (`ssh.password`, `secret(4096)`).
const MAX_PASSWORD_BYTES: usize = host::MAX_SECRET_BYTES;

/// The settings a session configuration needs (docs/data-model.md, section
/// 16.1; read under the mutex, used after).
struct SessionDefaults {
    term: String,
    keepalive_s: u32,
    record_history: bool,
    default_theme: Value,
    os_detection_enabled: bool,
    os_detection_script: String,
}

impl SessionDefaults {
    fn read(settings: &Settings) -> Self {
        let get = |key: &str| settings.get(key).unwrap_or(Value::Null);
        let script = get("terminal.os_detection_script");
        Self {
            term: get("terminal.term")
                .as_str()
                .unwrap_or("xterm-256color")
                .to_owned(),
            keepalive_s: get("terminal.keepalive_s")
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(30),
            record_history: get("terminal.record_history").as_bool().unwrap_or(true),
            default_theme: get("terminal.default_theme"),
            os_detection_enabled: get("terminal.os_detection_enabled")
                .as_bool()
                .unwrap_or(true),
            os_detection_script: script
                .as_str()
                .filter(|script| !script.trim().is_empty())
                .unwrap_or(os_detect::DEFAULT_SCRIPT)
                .to_owned(),
        }
    }
}

/// A built-in theme reference, `None` for anything else. Stage 2 knows no
/// `terminal_theme` records, so a `record_id` reference is a missing record
/// and falls back (docs/data-model.md, section 8.5).
fn builtin_theme(value: Option<&Value>) -> Option<Value> {
    let slug = value?.get("builtin")?.as_str()?;
    BUILTIN_THEMES
        .contains(&slug)
        .then(|| json!({ "builtin": slug }))
}

/// `host.terminal_theme` ?? `terminal.default_theme` ?? `graphite`.
fn effective_theme(host_theme: Option<&Value>, default_theme: &Value) -> Value {
    builtin_theme(host_theme)
        .or_else(|| builtin_theme(Some(default_theme)))
        .unwrap_or_else(|| json!({ "builtin": "graphite" }))
}

fn ssh_block(data: &Map<String, Value>) -> Option<&Map<String, Value>> {
    data.get("ssh").and_then(Value::as_object)
}

fn uuid_member(map: Option<&Map<String, Value>>, key: &str) -> Option<Uuid> {
    map?.get(key)
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

fn port_member(map: Option<&Map<String, Value>>) -> Option<u16> {
    map?.get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port >= 1)
}

fn username_member(map: Option<&Map<String, Value>>) -> Option<String> {
    map?.get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The host's `os` member, when it is well-formed.
fn host_os(data: &Map<String, Value>) -> Option<HostOs> {
    data.get("os")
        .and_then(|value| serde_json::from_value::<HostOs>(value.clone()).ok())
}

/// Effective port of a host: own, inherited, else 22.
fn effective_port(
    index: &Index,
    tree: &crate::model::tree::Tree,
    data: &Map<String, Value>,
) -> u16 {
    port_member(ssh_block(data))
        .or_else(|| {
            index
                .inherited_for_host(tree, data)
                .port
                .map(|port| port.value)
        })
        .unwrap_or(22)
}

/// Readable known host entries, ordered by record id.
fn known_entries(index: &Index) -> Vec<(Uuid, Entry)> {
    let mut entries: Vec<(Uuid, Entry)> = index
        .of_kind(RecordKind::KnownHost)
        .filter_map(|record| known_host::read(record.data()).map(|entry| (record.record_id, entry)))
        .collect();
    entries.sort_by_key(|(id, _)| *id);
    entries
}

/// Live saved hosts with their address and effective port (`host_ids` of
/// the Known Hosts screen).
fn host_targets(index: &Index) -> Vec<(Uuid, String, u16)> {
    let tree = index.tree();
    let mut targets: Vec<(Uuid, String, u16)> = index
        .of_kind(RecordKind::Host)
        .filter_map(|record| {
            let data = record.data();
            let address = data.get("address").and_then(Value::as_str)?;
            Some((
                record.record_id,
                address.to_owned(),
                effective_port(index, &tree, data),
            ))
        })
        .collect();
    targets.sort_by_key(|(id, _, _)| *id);
    targets
}

fn known_host_view(
    vault_id: Uuid,
    record: &IndexedRecord,
    entry: &Entry,
    targets: &[(Uuid, String, u16)],
) -> KnownHostView {
    let hashed = matches!(
        known_hosts::classify(&entry.host),
        known_hosts::HostKind::Hashed { .. }
    );
    let host_ids = targets
        .iter()
        .filter(|(_, address, port)| {
            known_hosts::entry_matches(&entry.host, entry.port, address, *port)
        })
        .map(|(id, _, _)| *id)
        .collect();
    KnownHostView {
        vault_id,
        record_id: record.record_id,
        created_at: uuid_v7_timestamp(&record.record_id),
        modified_at: format_unix_ms(record.payload.hlc.ms()),
        host: entry.host.clone(),
        port: if hashed { None } else { entry.port },
        hashed,
        display_address: (!hashed)
            .then(|| known_hosts::display_address(&entry.host, entry.port.unwrap_or(22))),
        key_type: entry.key_type.clone(),
        fingerprint: known_hosts::fingerprint_sha256(&entry.public_key).unwrap_or_default(),
        revoked: entry.revoked,
        source: entry.source,
        comment: entry.comment.clone(),
        host_ids,
    }
}

/// The view of one known host record.
fn known_host_view_of(session: &Session, record_id: &Uuid) -> Result<KnownHostView, CoreError> {
    let index = &session.index;
    let record = index
        .get_kind(record_id, RecordKind::KnownHost)
        .ok_or(CoreError::Internal("known host missing"))?;
    let entry =
        known_host::read(record.data()).ok_or(CoreError::Internal("known host unreadable"))?;
    Ok(known_host_view(
        session.ring.vault_id(),
        record,
        &entry,
        &host_targets(index),
    ))
}

/// A live record of `kind`, or `not_found`.
fn live<'a>(
    session: &'a Session,
    record_id: &Uuid,
    kind: RecordKind,
) -> Result<&'a IndexedRecord, CoreError> {
    session
        .index
        .get_kind(record_id, kind)
        .ok_or(CoreError::NotFound)
}

/// Distinct ids in their first order.
fn distinct(ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

/// The configuration of the saved host `record_id` alone: its effective
/// address, port, credentials and session settings, without its route
/// (no hops, no proxy; [`Core::session_config_impl`] adds them for the
/// target and uses this for each hop).
fn host_session_config(
    session: &Session,
    defaults: &SessionDefaults,
    entries: &[(Uuid, Entry)],
    vault_id: Uuid,
    record_id: &Uuid,
) -> Result<SessionConfig, CoreError> {
    let record = live(session, record_id, RecordKind::Host)?;
    let data = record.data();
    let index = &session.index;
    let tree = index.tree();
    let ssh = ssh_block(data);
    let inherited = index.inherited_for_host(&tree, data);
    let address = data
        .get("address")
        .and_then(Value::as_str)
        .filter(|address| !address.is_empty())
        .ok_or(CoreError::Damaged)?
        .to_owned();
    let port = port_member(ssh)
        .or_else(|| inherited.port.as_ref().map(|port| port.value))
        .unwrap_or(22);
    // Data-model 8.3: the host's own value, else its own
    // identity's, else the groups' (each level with its identity).
    let own_identity = own_identity(session, data);
    let username = username_member(ssh)
        .or_else(|| own_identity.and_then(identity::username).map(str::to_owned))
        .or_else(|| inherited.username.map(|name| name.value));
    let password =
        effective_password(session, data).map(|password| SecretString::from(password.to_owned()));
    let os = host_os(data);
    let os_detection_script = (defaults.os_detection_enabled && os.is_none())
        .then(|| defaults.os_detection_script.clone());
    Ok(SessionConfig {
        host: Some(RecordRef {
            vault_id,
            record_id: *record_id,
        }),
        title: host::title(data).to_owned(),
        port,
        telnet_port: data.get("telnet").and_then(Value::as_object).map(|telnet| {
            telnet
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port > 0)
                .unwrap_or(TELNET_PORT)
        }),
        username,
        password,
        key: session_key(session, data),
        keepalive_s: ssh
            .and_then(|ssh| ssh.get("keepalive_s"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value <= 3600)
            .unwrap_or(defaults.keepalive_s),
        term: defaults.term.clone(),
        agent_forwarding: ssh
            .and_then(|ssh| ssh.get("agent_forwarding"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        agent_forward_system: ssh
            .and_then(|ssh| ssh.get("agent_forward_system"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        agent_confirm: ssh
            .and_then(|ssh| ssh.get("agent_confirm"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        env: session_env(data),
        charset: charset_of(data),
        backspace: backspace_of(data),
        terminal_theme: effective_theme(data.get("terminal_theme"), &defaults.default_theme),
        record_history: record_history_of(data, defaults.record_history),
        os_detection_script,
        os,
        known_key_types: known_hosts::known_key_types(entries, &address, port),
        allow_legacy_algorithms: ssh
            .and_then(|ssh| ssh.get("allow_legacy_algorithms"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        can_save_password: true,
        jump_hosts: Vec::new(),
        route_problem: None,
        proxy: None,
        startup_snippet: uuid_member(ssh, "startup_snippet_id")
            .and_then(|id| index.get_kind(&id, RecordKind::Snippet))
            .and_then(|record| record.data().get("script")?.as_str().map(str::to_owned))
            .filter(|script| !script.trim().is_empty())
            .map(|script| StartupSnippet {
                variables: crate::snippet::variables(&script),
                script,
            }),
        address,
    })
}

/// Most hops of a chain (docs/data-model.md, section 7.3).
const MAX_HOPS: usize = 8;

/// The hops of a host's effective `jump_chain` (own, else inherited; `[]`
/// stops the lookup): live hosts of this vault in chain order. A host is
/// never its own hop (a bastion inside the group whose chain names it).
fn route_hops(
    session: &Session,
    record_id: &Uuid,
    data: &Map<String, Value>,
) -> Result<Vec<Uuid>, RouteProblem> {
    // An entry that is not a host id (data that bypassed the model) is a
    // missing hop: dropping it would shorten the route.
    let own: Option<Vec<Option<Uuid>>> = ssh_block(data)
        .and_then(|ssh| ssh.get("jump_chain"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().and_then(|raw| Uuid::parse_str(raw).ok()))
                .collect()
        });
    if let Some(index) = own
        .as_ref()
        .and_then(|chain| chain.iter().position(Option::is_none))
    {
        return Err(RouteProblem::MissingHop { index });
    }
    let own: Option<Vec<Uuid>> = own.map(|chain| chain.into_iter().flatten().collect());
    let chain: Vec<Uuid> = match own {
        Some(chain) => chain,
        None => {
            let tree = session.index.tree();
            session
                .index
                .inherited_for_host(&tree, data)
                .jump_chain
                .map(|chain| chain.value)
                .unwrap_or_default()
        }
    };
    let chain: Vec<Uuid> = chain.into_iter().filter(|hop| hop != record_id).collect();
    if chain.len() > MAX_HOPS {
        return Err(RouteProblem::TooManyHops);
    }
    for (index, hop) in chain.iter().enumerate() {
        if chain[..index].contains(hop) {
            return Err(RouteProblem::RepeatedHop { index });
        }
        if session.index.get_kind(hop, RecordKind::Host).is_none() {
            return Err(RouteProblem::MissingHop { index });
        }
    }
    Ok(chain)
}

/// `host.ssh.proxy` with its password (data-model 7.5).
fn session_proxy(data: &Map<String, Value>) -> Option<SessionProxy> {
    let proxy = ssh_block(data)?.get("proxy")?.as_object()?;
    let kind = match proxy.get("type")?.as_str()? {
        "http" => ProxyType::Http,
        "socks5" => ProxyType::Socks5,
        _ => return None,
    };
    Some(SessionProxy {
        kind,
        address: proxy.get("address")?.as_str()?.to_owned(),
        port: proxy
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port > 0)?,
        username: proxy
            .get("username")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
        password: host::secret_value(data, "ssh.proxy.password")
            .map(|password| SecretString::from(password.to_owned())),
    })
}

/// The effective password of a host (data-model 8.3): `host.ssh.password
/// ?? identity.password`, where identity = first(identity_id): a host that
/// names an identity uses that one, even when it is missing (no silent
/// fallback to a group's credentials), else the nearest group's. A
/// connection authenticates with it and the phone's password key types it
/// (D220, D223).
fn effective_password<'a>(session: &'a Session, data: &'a Map<String, Value>) -> Option<&'a str> {
    let index = &session.index;
    let linked = if uuid_member(ssh_block(data), "identity_id").is_some() {
        own_identity(session, data)
    } else {
        let tree = index.tree();
        index
            .inherited_for_host(&tree, data)
            .identity_id
            .and_then(|id| index.get_kind(&id.value, RecordKind::Identity))
            .map(|record| record.data())
    };
    host::secret_value(data, "ssh.password").or_else(|| linked.and_then(identity::password))
}

/// The identity a host's own `ssh.identity_id` names (`idf(0)`,
/// docs/data-model.md, section 8.3).
fn own_identity<'a>(
    session: &'a Session,
    data: &Map<String, Value>,
) -> Option<&'a Map<String, Value>> {
    uuid_member(ssh_block(data), "identity_id")
        .and_then(|id| session.index.get_kind(&id, RecordKind::Identity))
        .map(|record| record.data())
}

/// The effective key of a host (data-model 8.3): own `ssh.key_id`, else its
/// own identity's, else the inherited one (a group's or its identity's); a
/// reference to a missing (or unusable) key counts as unset at connect time.
fn session_key(session: &Session, data: &Map<String, Value>) -> Option<SessionKey> {
    let index = &session.index;
    let key_id = uuid_member(ssh_block(data), "key_id")
        .or_else(|| own_identity(session, data).and_then(identity::key_id))
        .or_else(|| {
            let tree = index.tree();
            index
                .inherited_for_host(&tree, data)
                .key_id
                .map(|key| key.value)
        })?;
    let record = index.get_kind(&key_id, RecordKind::Key)?;
    let key_data = record.data();
    if key::source(key_data) != "stored" {
        return None;
    }
    let private_key = key::private_key(key_data)?;
    let label = match key::label(key_data) {
        "" => key::public_key(key_data)
            .and_then(|line| line.split_whitespace().next())
            .unwrap_or("key")
            .to_owned(),
        label => label.to_owned(),
    };
    Some(SessionKey {
        key: RecordRef {
            vault_id: session.ring.vault_id(),
            record_id: key_id,
        },
        label,
        private_key: SecretString::from(private_key.to_owned()),
        passphrase: key::passphrase(key_data).map(|value| SecretString::from(value.to_owned())),
        can_save_passphrase: true,
    })
}

/// A `stored` key record as a session key (its label, else the public
/// key's type).
fn key_record_as_session_key(session: &Session, key_id: Uuid) -> Option<SessionKey> {
    let record = session.index.get_kind(&key_id, RecordKind::Key)?;
    let key_data = record.data();
    if key::source(key_data) != "stored" {
        return None;
    }
    let private_key = key::private_key(key_data)?;
    let label = match key::label(key_data) {
        "" => key::public_key(key_data)
            .and_then(|line| line.split_whitespace().next())
            .unwrap_or("key")
            .to_owned(),
        label => label.to_owned(),
    };
    Some(SessionKey {
        key: RecordRef {
            vault_id: session.ring.vault_id(),
            record_id: key_id,
        },
        label,
        private_key: SecretString::from(private_key.to_owned()),
        passphrase: key::passphrase(key_data).map(|value| SecretString::from(value.to_owned())),
        can_save_passphrase: false,
    })
}

impl Core {
    /// The Keychain keys the built-in agent offers to agent forwarding
    /// (stage 5, D60): every stored private key of the vault, as stored
    /// (possibly encrypted) with its saved passphrase. The caller decrypts
    /// them, without the core mutex, and drops those it cannot open
    /// without asking; it wipes them on lock.
    pub fn agent_keys(&self) -> Result<Vec<SessionKey>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let mut keys: Vec<SessionKey> = session
            .index
            .of_kind(RecordKind::Key)
            .filter_map(|record| key_record_as_session_key(session, record.record_id))
            .collect();
        keys.sort_by(|a, b| {
            fold(&a.label)
                .cmp(&fold(&b.label))
                .then_with(|| a.key.record_id.cmp(&b.key.record_id))
        });
        Ok(keys)
    }

    /// The password the phone's password key types into a session to the
    /// saved host `host` (D220, D223): the host's effective password, the
    /// one a connection authenticates with (its own, else its identity's,
    /// data-model 8.3); `not_found` without one.
    pub fn host_password(&self, host: RecordRef) -> Result<SecretString, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(host.vault_id))?;
        let record = live(session, &host.record_id, RecordKind::Host)?;
        effective_password(session, record.data())
            .map(|password| SecretString::from(password.to_owned()))
            .ok_or(CoreError::NotFound)
    }

    /// Whether the saved host `host` has an effective password (the
    /// password key's state), without reading it out.
    pub fn host_password_set(&self, host: RecordRef) -> Result<bool, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        check_vault(session, Some(host.vault_id))?;
        let record = live(session, &host.record_id, RecordKind::Host)?;
        Ok(effective_password(session, record.data()).is_some())
    }
}

/// `ssh.env` as name and value pairs.
fn session_env(data: &Map<String, Value>) -> Vec<(String, SecretString)> {
    ssh_block(data)
        .and_then(|ssh| ssh.get("env"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let name = item.get("name").and_then(Value::as_str)?;
                    let value = item.get("value").and_then(Value::as_str)?;
                    Some((name.to_owned(), SecretString::from(value.to_owned())))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn backspace_of(data: &Map<String, Value>) -> Backspace {
    data.get("backspace")
        .and_then(|value| serde_json::from_value::<Backspace>(value.clone()).ok())
        .unwrap_or_default()
}

fn charset_of(data: &Map<String, Value>) -> String {
    data.get("charset")
        .and_then(Value::as_str)
        .filter(|charset| !charset.is_empty())
        .map_or_else(|| "utf-8".to_owned(), str::to_lowercase)
}

/// Record history: the host override unless `default`, else the setting.
fn record_history_of(data: &Map<String, Value>, setting: bool) -> bool {
    match data.get("record_history").and_then(Value::as_str) {
        Some("on") => true,
        Some("off") => false,
        _ => setting,
    }
}

/// The effective `record_history` of a session now: the linked host's
/// override for a host of the personal vault, else the setting
/// (docs/security.md, section 10.1, rule 4). A personal host that no longer
/// exists records nothing: history is kept only for saved hosts (stage 2b
/// decision), and an entry of a removed host would show in no History tab.
fn current_record_history(session: &Session, host: Option<RecordRef>, setting: bool) -> bool {
    let personal = session.ring.vault_id();
    match host {
        Some(host) if host.vault_id == personal => session
            .index
            .get_kind(&host.record_id, RecordKind::Host)
            .is_some_and(|record| record_history_of(record.data(), setting)),
        _ => setting,
    }
}

/// The oldest end (or start, while open) a log entry keeps after `days`
/// of retention, as a stored timestamp (empty: nothing expires).
fn retention_cutoff(days: u64, now: SystemTime) -> String {
    now.checked_sub(std::time::Duration::from_secs(days.saturating_mul(86_400)))
        .map(format_timestamp)
        .unwrap_or_default()
}

/// `logs.retention_days` (default 90).
fn retention_days(settings: &Settings) -> u64 {
    settings
        .get("logs.retention_days")
        .and_then(|value| value.as_u64())
        .unwrap_or(90)
}

/// An unsaved log entry whose end (or start) is older than `cutoff`.
fn expired(data: &Map<String, Value>, cutoff: &str) -> bool {
    let last = connection_log::ended_at(data).unwrap_or_else(|| connection_log::started_at(data));
    !connection_log::saved(data) && last < cutoff
}

/// `terminal.record_history` (default on).
fn record_history_setting(settings: &Settings) -> bool {
    settings
        .get("terminal.record_history")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

/// A `history_entry` matches a `history_list`/`history_clear` host filter.
fn same_link(data: &Map<String, Value>, host: Option<RecordRef>) -> bool {
    history_entry::host(data) == host
}

impl Core {
    /// Loads a lazy class on its first use in this unlocked session. Rows of
    /// older key epochs found there are re-sealed like the eager classes at
    /// unlock (best effort: they stay readable through the chain).
    pub(super) fn load_lazy(
        &self,
        parts: &mut Parts<'_>,
        class: u8,
        now: SystemTime,
    ) -> Result<bool, CoreError> {
        if !parts.session.ensure_class(parts.store, class)? {
            return Ok(false);
        }
        match parts.session.reseal_older(parts.store, now) {
            Ok(ids) if !ids.is_empty() => {
                let mut classes: Vec<u8> = ids
                    .iter()
                    .filter_map(|id| parts.session.index.get(id))
                    .map(|record| record.kind.class())
                    .collect();
                classes.sort_unstable();
                classes.dedup();
                let vault_id = parts.session.ring.vault_id();
                self.emit_with(
                    parts.v,
                    super::dto::CoreEventKind::RecordsChanged {
                        vault_id,
                        record_ids: ids,
                        classes,
                    },
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(code = error.code(), "re-seal deferred"),
        }
        Ok(true)
    }

    /// History entries may be read: loads the class if needed.
    fn ensure_history(&self, parts: &mut Parts<'_>, now: SystemTime) -> Result<(), CoreError> {
        self.load_lazy(parts, classes::HISTORY, now).map(drop)
    }

    /// Removes rows in one transaction: deleted at once while the vault is
    /// not bound and the record never synced, otherwise tombstoned
    /// (docs/storage-format.md, section 4.6). Emits `records:changed`.
    fn remove_rows(
        &self,
        parts: &mut Parts<'_>,
        ids: &[(Uuid, RecordKind)],
        now: SystemTime,
    ) -> Result<(), CoreError> {
        self.remove_rows_with(parts, ids, now, true)
    }

    /// [`Core::remove_rows`]; `trash: false` for log retention (expiry, not
    /// a deletion: nothing goes to Recently deleted, data-model 13.3).
    fn remove_rows_with(
        &self,
        parts: &mut Parts<'_>,
        ids: &[(Uuid, RecordKind)],
        now: SystemTime,
        trash: bool,
    ) -> Result<(), CoreError> {
        let vault_id = parts.session.ring.vault_id();
        let bound = parts.store.vault(&vault_id)?.is_some_and(|row| row.bound);
        let mut deletions = Vec::new();
        let mut tombstones = Vec::new();
        for (id, kind) in ids {
            let Some(previous) = parts.store.record(&vault_id, id)? else {
                parts.session.index.remove(id);
                continue;
            };
            if !bound && previous.synced_revision == 0 {
                deletions.push((*id, previous.class));
            } else {
                tombstones.push(parts.session.prepare_tombstone(*kind, &previous, now)?);
            }
        }
        self.commit_records_with(parts, Vec::new(), deletions, tombstones, trash)
    }

    /// Writes one record version with the given data.
    pub(super) fn write_record(
        &self,
        parts: &mut Parts<'_>,
        kind: RecordKind,
        record_id: Uuid,
        data: Map<String, Value>,
        now: SystemTime,
    ) -> Result<(), CoreError> {
        let write = parts
            .session
            .prepare_write(parts.store, kind, record_id, data, now)?;
        self.commit_records(parts, vec![write], Vec::new(), Vec::new())
    }

    /// Loads the connection log on its first use in this unlocked session,
    /// then writes the ends held while locked and gives entries of this
    /// device left open by an earlier run `end_reason = unknown`
    /// (docs/data-model.md, section 13.3).
    fn ensure_connlog(&self, parts: &mut Parts<'_>, now: SystemTime) -> Result<(), CoreError> {
        if !self.load_lazy(parts, classes::CONNECTION_LOG, now)? {
            return Ok(());
        }
        let device = parts.session.device_id;
        let pending: Vec<PendingEnd> = parts.connlog.pending().to_vec();
        let mut updates: Vec<(Uuid, Map<String, Value>)> = Vec::new();
        for end in &pending {
            if let Some(record) = parts
                .session
                .index
                .get_kind(&end.log_id, RecordKind::ConnectionLogEntry)
            {
                let mut data = record.data().clone();
                connection_log::set_end(
                    &mut data,
                    &end.ended_at,
                    connection_log::reason_name(end.reason),
                    end.exit_status,
                );
                updates.push((end.log_id, data));
            }
        }
        for record in parts.session.index.of_kind(RecordKind::ConnectionLogEntry) {
            let data = record.data();
            if connection_log::device_id(data) == Some(device)
                && connection_log::is_open(data)
                && !parts.connlog.is_active(&record.record_id)
            {
                let mut data = data.clone();
                connection_log::set_unknown_end(&mut data);
                updates.push((record.record_id, data));
            }
        }
        let unknown = updates.len().saturating_sub(pending.len());
        let mut writes = Vec::with_capacity(updates.len());
        for (record_id, data) in updates {
            writes.push(parts.session.prepare_write(
                parts.store,
                RecordKind::ConnectionLogEntry,
                record_id,
                data,
                now,
            )?);
        }
        if !writes.is_empty() {
            self.commit_records(parts, writes, Vec::new(), Vec::new())?;
        }
        for end in &pending {
            parts.connlog.ended(&end.log_id);
        }
        if unknown > 0 || !pending.is_empty() {
            tracing::info!(
                pending = pending.len(),
                unknown,
                "connection log entries closed after unlock"
            );
        }
        Ok(())
    }

    /// Called right after an unlock: writes the connection log ends held
    /// while locked (this loads the class early only when there are some).
    pub(super) fn connlog_after_unlock(&self, inner: &mut Inner, now: SystemTime) {
        if !inner.connlog.has_pending() {
            return;
        }
        let Ok(mut parts) = parts(inner) else {
            return;
        };
        if let Err(error) = self.ensure_connlog(&mut parts, now) {
            tracing::warn!(code = error.code(), "connection log ends deferred");
        }
    }

    pub(super) fn session_config_impl(
        &self,
        target: &SessionTarget,
    ) -> Result<SessionConfig, CoreError> {
        let inner = self.inner();
        let defaults = SessionDefaults::read(&inner.settings);
        let session = Self::require_session(&inner)?;
        let entries = known_entries(&session.index);
        match target {
            SessionTarget::Host {
                vault_id,
                record_id,
            } => {
                check_vault(session, Some(*vault_id))?;
                let mut config =
                    host_session_config(session, &defaults, &entries, *vault_id, record_id)?;
                let record = live(session, record_id, RecordKind::Host)?;
                let data = record.data();
                config.proxy = session_proxy(data);
                // A proxy that is set but unreadable never means "direct".
                let proxy_set = ssh_block(data)
                    .is_some_and(|ssh| ssh.get("proxy").is_some_and(|proxy| !proxy.is_null()));
                if proxy_set && config.proxy.is_none() {
                    config.route_problem = Some(RouteProblem::InvalidProxy);
                    return Ok(config);
                }
                match route_hops(session, record_id, data) {
                    Ok(hops) => {
                        for hop in hops {
                            let mut hop =
                                host_session_config(session, &defaults, &entries, *vault_id, &hop)?;
                            // A hop runs no session: no detection, no env,
                            // no startup snippet.
                            hop.os_detection_script = None;
                            hop.env.clear();
                            hop.startup_snippet = None;
                            hop.agent_forwarding = false;
                            hop.agent_forward_system = false;
                            config.jump_hosts.push(hop);
                        }
                    }
                    Err(problem) => config.route_problem = Some(problem),
                }
                Ok(config)
            }
            SessionTarget::Quick {
                address,
                port,
                username,
            } => {
                let mut errors: Vec<FieldError> = Vec::new();
                let address = match fields::address(address) {
                    Ok(Some(address)) => Some(address),
                    Ok(None) => {
                        errors.push(FieldError::new("address", ValidationReason::Required));
                        None
                    }
                    Err(reason) => {
                        errors.push(FieldError::new("address", reason));
                        None
                    }
                };
                let port = match port {
                    Some(0) => {
                        errors.push(FieldError::new("port", ValidationReason::OutOfRange));
                        22
                    }
                    Some(port) => *port,
                    None => 22,
                };
                let username = match username.as_deref().map(|name| text::string(name, 256)) {
                    None => None,
                    Some(Ok(name)) => name,
                    Some(Err(reason)) => {
                        errors.push(FieldError::new("username", reason));
                        None
                    }
                };
                let address = match address {
                    Some(address) if errors.is_empty() => address,
                    _ => return Err(CoreError::Validation(errors)),
                };
                Ok(SessionConfig {
                    host: None,
                    title: address.clone(),
                    port,
                    telnet_port: None,
                    username,
                    password: None,
                    key: None,
                    keepalive_s: defaults.keepalive_s,
                    term: defaults.term,
                    agent_forwarding: false,
                    agent_forward_system: false,
                    agent_confirm: false,
                    env: Vec::new(),
                    charset: "utf-8".to_owned(),
                    backspace: Backspace::Default,
                    terminal_theme: effective_theme(None, &defaults.default_theme),
                    record_history: defaults.record_history,
                    os_detection_script: None,
                    os: None,
                    known_key_types: known_hosts::known_key_types(&entries, &address, port),
                    allow_legacy_algorithms: false,
                    can_save_password: false,
                    jump_hosts: Vec::new(),
                    route_problem: None,
                    proxy: None,
                    startup_snippet: None,
                    address,
                })
            }
        }
    }

    pub(super) fn known_host_check_impl(
        &self,
        check: &HostKeyCheck,
    ) -> Result<HostKeyStatus, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let blob = known_hosts::decode_blob(&check.public_key)
            .ok_or_else(|| CoreError::field("public_key", ValidationReason::Invalid))?;
        let entries = known_entries(&session.index);
        Ok(known_hosts::check(
            &entries,
            &check.address,
            check.port,
            &check.key_type,
            &blob,
        ))
    }

    /// Stores a validated entry unless an entry with the same natural key
    /// exists; returns the record id.
    fn add_known_host(
        &self,
        parts: &mut Parts<'_>,
        entry: &Entry,
        now: SystemTime,
    ) -> Result<Uuid, CoreError> {
        if let Some((existing, _)) = known_entries(&parts.session.index)
            .into_iter()
            .find(|(_, stored)| known_host::same_natural_key(stored, entry))
        {
            return Ok(existing);
        }
        let record_id = parts.ids.next(now);
        self.write_record(
            parts,
            RecordKind::KnownHost,
            record_id,
            known_host::to_data(entry),
            now,
        )?;
        tracing::info!(record_id = %record_id, "known host saved");
        Ok(record_id)
    }

    pub(super) fn known_host_accept_impl(
        &self,
        check: &HostKeyCheck,
    ) -> Result<KnownHostView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        // "Accept and save" stores the address as connected: a plain entry,
        // never a pattern (a saved address may contain `*` or `?`).
        if known_hosts::classify(check.address.trim()) != known_hosts::HostKind::Plain {
            return Err(CoreError::field("host", ValidationReason::Invalid));
        }
        let entry = known_host::validate(EntryInput {
            host: &check.address,
            port: Some(check.port),
            key_type: &check.key_type,
            public_key: &check.public_key,
            revoked: false,
            source: KnownHostSource::Tofu,
            comment: None,
        })
        .map_err(CoreError::Validation)?;
        let record_id = self.add_known_host(&mut parts, &entry, now)?;
        known_host_view_of(parts.session, &record_id)
    }

    pub(super) fn known_host_replace_impl(
        &self,
        record_id: Uuid,
        check: &HostKeyCheck,
    ) -> Result<KnownHostView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let record = live(parts.session, &record_id, RecordKind::KnownHost)?;
        let stored = known_host::read(record.data()).ok_or(CoreError::NotFound)?;
        if stored.revoked {
            return Err(CoreError::WrongState);
        }
        // Only an entry for exactly this address and port is rewritten. A
        // wildcard pattern (or an entry for another host) covers other hosts
        // too: one decision about this connection must not change their
        // trust, so this host gets its own plain entry and the pattern stays.
        let exact =
            matches!(
                known_hosts::classify(&stored.host),
                known_hosts::HostKind::Plain | known_hosts::HostKind::Hashed { .. }
            ) && known_hosts::entry_matches(&stored.host, stored.port, &check.address, check.port);
        if !exact {
            if known_hosts::classify(check.address.trim()) != known_hosts::HostKind::Plain {
                return Err(CoreError::field("host", ValidationReason::Invalid));
            }
            let entry = known_host::validate(EntryInput {
                host: &check.address,
                port: Some(check.port),
                key_type: &check.key_type,
                public_key: &check.public_key,
                revoked: false,
                source: KnownHostSource::Replaced,
                comment: None,
            })
            .map_err(CoreError::Validation)?;
            let added = self.add_known_host(&mut parts, &entry, now)?;
            tracing::info!(record_id = %added, "known host key replaced for one host");
            return known_host_view_of(parts.session, &added);
        }
        let record = live(parts.session, &record_id, RecordKind::KnownHost)?;
        let entry = known_host::validate(EntryInput {
            host: &stored.host,
            port: stored.port,
            key_type: &check.key_type,
            public_key: &check.public_key,
            revoked: false,
            source: KnownHostSource::Replaced,
            comment: stored.comment.as_deref(),
        })
        .map_err(CoreError::Validation)?;
        let mut data = record.data().clone();
        data.insert("key_type".into(), Value::String(entry.key_type));
        data.insert("public_key".into(), Value::String(entry.public_key));
        data.insert(
            "source".into(),
            json!(known_host::source_name(KnownHostSource::Replaced)),
        );
        self.write_record(&mut parts, RecordKind::KnownHost, record_id, data, now)?;
        tracing::info!(record_id = %record_id, "known host key replaced");
        known_host_view_of(parts.session, &record_id)
    }

    pub(super) fn known_host_add_impl(
        &self,
        input: KnownHostAddInput,
    ) -> Result<KnownHostView, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let entry = known_host::validate(EntryInput {
            host: &input.host,
            port: input.port,
            key_type: &input.key_type,
            public_key: &input.public_key,
            revoked: input.revoked,
            source: KnownHostSource::Import,
            comment: input.comment.as_deref(),
        })
        .map_err(CoreError::Validation)?;
        let record_id = self.add_known_host(&mut parts, &entry, now)?;
        known_host_view_of(parts.session, &record_id)
    }

    pub(super) fn known_hosts_list_impl(&self) -> Result<Vec<KnownHostView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let index = &session.index;
        let targets = host_targets(index);
        let vault_id = session.ring.vault_id();
        let mut views: Vec<KnownHostView> = index
            .of_kind(RecordKind::KnownHost)
            .filter_map(|record| {
                known_host::read(record.data())
                    .map(|entry| known_host_view(vault_id, record, &entry, &targets))
            })
            .collect();
        // The folded address is computed once per entry, not per comparison.
        views.sort_by_cached_key(|view| {
            (
                view.hashed,
                view.display_address
                    .as_deref()
                    .map(fold)
                    .unwrap_or_default(),
                view.record_id,
            )
        });
        Ok(views)
    }

    pub(super) fn known_host_remove_impl(&self, input: RecordsRef) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        let ids = distinct(&input.record_ids);
        for id in &ids {
            live(parts.session, id, RecordKind::KnownHost)?;
        }
        let targets: Vec<(Uuid, RecordKind)> =
            ids.iter().map(|id| (*id, RecordKind::KnownHost)).collect();
        // Undo (UI 10.1 as hosts, 2.4): the last live payloads, memory only.
        let undo: Vec<(Uuid, crate::model::payload::Payload)> = ids
            .iter()
            .filter_map(|id| {
                parts
                    .session
                    .index
                    .get(id)
                    .map(|record| (*id, record.payload.clone()))
            })
            .collect();
        self.remove_rows(&mut parts, &targets, now)?;
        for (id, payload) in undo {
            parts
                .session
                .undo
                .put(id, RecordKind::KnownHost, payload, now);
        }
        tracing::info!(removed = ids.len(), "known hosts removed");
        Ok(())
    }

    pub(super) fn known_host_restore_impl(
        &self,
        input: RecordsRef,
    ) -> Result<Vec<KnownHostView>, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        parts.session.undo.purge(now);
        let ids = distinct(&input.record_ids);
        let mut restored = Vec::with_capacity(ids.len());
        for id in &ids {
            match parts.session.undo.get(id) {
                Some(entry)
                    if entry.kind == RecordKind::KnownHost
                        && parts.session.index.get(id).is_none() =>
                {
                    restored.push((*id, entry.payload.data.clone().unwrap_or_default()));
                }
                _ => return Err(CoreError::NotFound),
            }
        }
        let mut writes = Vec::with_capacity(restored.len());
        for (id, data) in restored {
            writes.push(parts.session.prepare_write(
                parts.store,
                RecordKind::KnownHost,
                id,
                data,
                now,
            )?);
        }
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        for id in &ids {
            parts.session.undo.remove(id);
        }
        tracing::info!(restored = ids.len(), "known hosts restored");
        ids.iter()
            .map(|id| known_host_view_of(parts.session, id))
            .collect()
    }

    pub(super) fn keys_list_impl(&self) -> Result<Vec<KeyView>, CoreError> {
        let inner = self.inner();
        let session = Self::require_session(&inner)?;
        let vault_id = session.ring.vault_id();
        let mut views: Vec<KeyView> = session
            .index
            .of_kind(RecordKind::Key)
            .map(|record| {
                keys::view(
                    vault_id,
                    record.record_id,
                    record.payload.hlc.ms(),
                    record.data(),
                )
            })
            .collect();
        views.sort_by(|a, b| {
            fold(&a.label)
                .cmp(&fold(&b.label))
                .then_with(|| a.record_id.cmp(&b.record_id))
        });
        Ok(views)
    }

    pub(super) fn key_view_of(session: &Session, record_id: &Uuid) -> Result<KeyView, CoreError> {
        let record = session
            .index
            .get_kind(record_id, RecordKind::Key)
            .ok_or(CoreError::Internal("key missing"))?;
        Ok(keys::view(
            session.ring.vault_id(),
            *record_id,
            record.payload.hlc.ms(),
            record.data(),
        ))
    }

    /// The live key with this fingerprint, if any (natural key).
    fn key_by_fingerprint(session: &Session, fingerprint: &str) -> Option<Uuid> {
        let mut ids: Vec<Uuid> = session
            .index
            .of_kind(RecordKind::Key)
            .filter(|record| {
                keys::record_fingerprint(record.data()).as_deref() == Some(fingerprint)
            })
            .map(|record| record.record_id)
            .collect();
        ids.sort();
        ids.first().copied()
    }

    pub(super) fn key_import_impl(
        &self,
        input: KeyImportInput,
    ) -> Result<KeyImportResult, CoreError> {
        {
            let inner = self.inner();
            let session = Self::require_session(&inner)?;
            check_vault(session, Some(input.vault_id))?;
        }
        // Convert, parse and check the passphrase without the mutex (the
        // source's key derivation, bcrypt-pbkdf). PEM and `.ppk` keys become
        // OpenSSH keys, encrypted again with their passphrase (stage 3).
        let mut errors: Vec<FieldError> = Vec::new();
        let prepared = keys::prepare(input.private_key.expose_secret(), input.passphrase.as_ref());
        let parsed = match prepared.and_then(|text| keys::parse(&text)) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                errors.push(error);
                None
            }
        };
        if let Some(parsed) = &parsed
            && let Err(error) = keys::check_passphrase(parsed, input.passphrase.as_ref())
        {
            errors.push(error);
        }
        let label = match &parsed {
            Some(parsed) => match keys::label_for(input.label.as_deref(), parsed) {
                Ok(label) => Some(label),
                Err(error) => {
                    errors.push(error);
                    None
                }
            },
            None => {
                if let Some(Err(reason)) = input
                    .label
                    .as_deref()
                    .map(|label| text::string(label, key::MAX_LABEL_CHARS))
                {
                    errors.push(FieldError::new("label", reason));
                }
                None
            }
        };
        let (Some(parsed), Some(label)) = (parsed, label) else {
            return Err(CoreError::Validation(errors));
        };
        if !errors.is_empty() {
            return Err(CoreError::Validation(errors));
        }
        let save: Option<SecretString> = input.passphrase.filter(|passphrase| {
            input.save_passphrase && parsed.encrypted && !passphrase.expose_secret().is_empty()
        });

        // An existing key with the same fingerprint is returned; it gets the
        // passphrase when it has none, the user asked to save it and the
        // passphrase opens the stored text.
        let check_existing = {
            let now = self.now();
            let mut guard = self.inner();
            let mut parts = parts(&mut guard)?;
            check_vault(parts.session, Some(input.vault_id))?;
            match Self::key_by_fingerprint(parts.session, &parsed.fingerprint) {
                None => {
                    let record_id = parts.ids.next(now);
                    let mut data = WipeOnDrop::new(key::new_stored(
                        &label,
                        &parsed.text,
                        &parsed.public_key,
                        save.as_ref().map(|value| value.expose_secret()),
                    ));
                    // A security key carries its FIDO2 member (data-model 9.3).
                    if let Some(fido2) = keys::fido2_member(&parsed.text) {
                        data.insert("fido2".into(), fido2);
                    }
                    self.write_record(
                        &mut parts,
                        RecordKind::Key,
                        record_id,
                        data.into_inner(),
                        now,
                    )?;
                    tracing::info!(record_id = %record_id, "key imported");
                    return Ok(KeyImportResult {
                        key: Self::key_view_of(parts.session, &record_id)?,
                        created: true,
                    });
                }
                Some(existing) => {
                    let data = live(parts.session, &existing, RecordKind::Key)?.data();
                    let stored_text =
                        key::private_key(data).map(|text| Zeroizing::new(text.to_owned()));
                    let wants = save.is_some()
                        && key::passphrase(data).is_none()
                        && key::source(data) == "stored";
                    match (wants, stored_text) {
                        (true, Some(stored_text)) => (existing, stored_text),
                        _ => {
                            return Ok(KeyImportResult {
                                key: Self::key_view_of(parts.session, &existing)?,
                                created: false,
                            });
                        }
                    }
                }
            }
        };
        let (existing, stored_text) = check_existing;
        let Some(passphrase) = save else {
            return Err(CoreError::Internal("passphrase missing"));
        };
        let opens = stored_text.as_str() == parsed.text.as_str()
            || keys::passphrase_opens(&stored_text, &passphrase).is_ok();
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let record = live(parts.session, &existing, RecordKind::Key)?;
        let unchanged = key::private_key(record.data()) == Some(stored_text.as_str())
            && key::passphrase(record.data()).is_none();
        if opens && unchanged {
            let mut data = WipeOnDrop::new(record.data().clone());
            key::set_passphrase(&mut data, Some(passphrase.expose_secret()));
            self.write_record(
                &mut parts,
                RecordKind::Key,
                existing,
                data.into_inner(),
                now,
            )?;
            tracing::info!(record_id = %existing, "key passphrase saved");
        }
        Ok(KeyImportResult {
            key: Self::key_view_of(parts.session, &existing)?,
            created: false,
        })
    }

    pub(super) fn save_prompted_password_impl(
        &self,
        host: RecordRef,
        password: SecretString,
    ) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(host.vault_id))?;
        let record = live(parts.session, &host.record_id, RecordKind::Host)?;
        let value = password.expose_secret();
        if value.is_empty() {
            return Ok(());
        }
        text::secret(value, MAX_PASSWORD_BYTES)
            .map_err(|reason| CoreError::field("ssh.password", reason))?;
        let mut data = WipeOnDrop::new(record.data().clone());
        let ssh = data
            .entry("ssh")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(ssh) = ssh.as_object_mut() else {
            return Err(CoreError::Damaged);
        };
        if let Some(mut old) = ssh.insert("password".into(), Value::String(value.to_owned())) {
            crate::model::wipe::wipe_value(&mut old);
        }
        self.write_record(
            &mut parts,
            RecordKind::Host,
            host.record_id,
            data.into_inner(),
            now,
        )?;
        tracing::info!(record_id = %host.record_id, "prompted password saved");
        Ok(())
    }

    pub(super) fn save_key_passphrase_impl(
        &self,
        key_ref: RecordRef,
        passphrase: SecretString,
    ) -> Result<(), CoreError> {
        let text = {
            let inner = self.inner();
            let session = Self::require_session(&inner)?;
            check_vault(session, Some(key_ref.vault_id))?;
            let record = live(session, &key_ref.record_id, RecordKind::Key)?;
            Zeroizing::new(
                key::private_key(record.data())
                    .ok_or_else(|| CoreError::field("passphrase", ValidationReason::NotAllowed))?
                    .to_owned(),
            )
        };
        if passphrase.expose_secret().is_empty() {
            return Err(CoreError::field("passphrase", ValidationReason::Required));
        }
        keys::passphrase_opens(&text, &passphrase)
            .map_err(|error| CoreError::Validation(vec![error]))?;
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let record = live(parts.session, &key_ref.record_id, RecordKind::Key)?;
        if key::private_key(record.data()) != Some(text.as_str()) {
            // The key was replaced meanwhile: the checked passphrase does not apply.
            return Err(CoreError::WrongState);
        }
        let mut data = WipeOnDrop::new(record.data().clone());
        key::set_passphrase(&mut data, Some(passphrase.expose_secret()));
        self.write_record(
            &mut parts,
            RecordKind::Key,
            key_ref.record_id,
            data.into_inner(),
            now,
        )?;
        tracing::info!(record_id = %key_ref.record_id, "key passphrase saved");
        Ok(())
    }

    pub(super) fn host_os_detected_impl(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(host.vault_id))?;
        let record = live(parts.session, &host.record_id, RecordKind::Host)?;
        let os = os.sanitized();
        let current = host_os(record.data());
        let differs = match &current {
            None => true,
            Some(current) => current.family != os.family || current.distro != os.distro,
        };
        if !differs {
            return Ok(false);
        }
        let value = HostOs {
            family: os.family,
            distro: os.distro,
            version: os.version,
            source,
            detected_at: format_timestamp(now),
        };
        let mut data = WipeOnDrop::new(record.data().clone());
        data.insert(
            "os".into(),
            serde_json::to_value(&value).map_err(|_| CoreError::Internal("os value"))?,
        );
        self.write_record(
            &mut parts,
            RecordKind::Host,
            host.record_id,
            data.into_inner(),
            now,
        )?;
        tracing::info!(record_id = %host.record_id, "host os detected");
        Ok(true)
    }

    pub(super) fn history_record_impl(
        &self,
        input: HistoryRecordInput,
    ) -> Result<HistoryDecision, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let setting = record_history_setting(&guard.settings);
        let Ok(mut parts) = parts(&mut guard) else {
            return Ok(HistoryDecision::Dropped(HistoryDropReason::Locked));
        };
        // The session's copy of the flag may be stale (the host's override or
        // the setting changed while it runs): both must allow recording now.
        let record_history =
            input.record_history && current_record_history(parts.session, input.host, setting);
        let command = match history::evaluate(&input.candidate, input.injected, record_history) {
            Ok(command) => Zeroizing::new(command),
            Err(reason) => return Ok(HistoryDecision::Dropped(reason)),
        };
        if parts
            .session
            .history_last
            .get(&input.session_id)
            .is_some_and(|last| last.as_str() == command.as_str())
        {
            return Ok(HistoryDecision::Dropped(HistoryDropReason::Duplicate));
        }
        let data = history_entry::new_typed(input.host, &command, &format_timestamp(now));
        let record_id = parts.ids.next(now);
        self.write_record(&mut parts, RecordKind::HistoryEntry, record_id, data, now)?;
        parts.session.history_last.insert(input.session_id, command);
        Ok(HistoryDecision::Recorded)
    }

    pub(super) fn history_session_end_impl(&self, session_id: Uuid) {
        let mut guard = self.inner();
        if let Ok(parts) = parts(&mut guard) {
            parts.session.history_last.remove(&session_id);
        }
    }

    pub(super) fn record_history_now_impl(
        &self,
        host: Option<RecordRef>,
    ) -> Result<bool, CoreError> {
        let inner = self.inner();
        let setting = record_history_setting(&inner.settings);
        let session = Self::require_session(&inner)?;
        Ok(current_record_history(session, host, setting))
    }

    pub(super) fn history_list_impl(
        &self,
        input: HistoryListInput,
    ) -> Result<Vec<HistoryEntryView>, CoreError> {
        let host = input.host;
        self.history_entries(input.text.as_deref(), input.limit, false, |data| {
            same_link(data, host)
        })
    }

    /// The History tab (UI 4.2): as `history_list`, with the entries of one
    /// command collapsed into its newest (run 3 polish, D64); each row
    /// lists the ids of the older ones, so deleting the row deletes them all.
    pub fn history_list_distinct(
        &self,
        input: HistoryListInput,
    ) -> Result<Vec<HistoryEntryView>, CoreError> {
        let host = input.host;
        self.history_entries(input.text.as_deref(), input.limit, true, |data| {
            same_link(data, host)
        })
    }

    /// The Shell History screen (UI 9.1): every entry, or those of `hosts`.
    pub fn history_search(
        &self,
        input: super::dto::HistorySearchInput,
    ) -> Result<Vec<HistoryEntryView>, CoreError> {
        let hosts = input.hosts;
        self.history_entries(input.text.as_deref(), input.limit, false, |data| {
            hosts.is_empty() || history_entry::host(data).is_some_and(|host| hosts.contains(&host))
        })
    }

    /// History entries `keep` selects, newest first, filtered by `text`;
    /// `distinct`: one row per command (its newest entry, the others in
    /// `older`), the limit counting rows.
    fn history_entries(
        &self,
        text: Option<&str>,
        limit: Option<u32>,
        distinct: bool,
        keep: impl Fn(&Map<String, Value>) -> bool,
    ) -> Result<Vec<HistoryEntryView>, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_history(&mut parts, now)?;
        let text = text
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(fold);
        let limit = limit.unwrap_or(HISTORY_LIST_DEFAULT).min(HISTORY_LIST_MAX) as usize;
        let mut entries: Vec<&IndexedRecord> = parts
            .session
            .index
            .of_kind(RecordKind::HistoryEntry)
            .filter(|record| keep(record.data()))
            .filter(|record| {
                text.as_deref()
                    .is_none_or(|text| fold(history_entry::command(record.data())).contains(text))
            })
            .collect();
        entries.sort_by(|a, b| {
            history_entry::executed_at(b.data())
                .cmp(history_entry::executed_at(a.data()))
                .then_with(|| b.record_id.cmp(&a.record_id))
        });
        let vault_id = parts.session.ring.vault_id();
        let view = |record: &IndexedRecord| HistoryEntryView {
            vault_id,
            record_id: record.record_id,
            command: history_entry::command(record.data()).to_owned(),
            executed_at: history_entry::executed_at(record.data()).to_owned(),
            host: history_entry::host(record.data()),
            older: Vec::new(),
        };
        if !distinct {
            return Ok(entries.into_iter().take(limit).map(view).collect());
        }
        let mut rows: Vec<HistoryEntryView> = Vec::new();
        let mut row_of: HashMap<&str, usize> = HashMap::new();
        for record in entries {
            let command = history_entry::command(record.data());
            match row_of.get(command) {
                Some(&row) => rows[row].older.push(record.record_id),
                // Past the limit only the older ids of shown rows count.
                None if rows.len() < limit => {
                    row_of.insert(command, rows.len());
                    rows.push(view(record));
                }
                None => {}
            }
        }
        Ok(rows)
    }

    pub(super) fn history_delete_impl(&self, input: RecordsRef) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(input.vault_id))?;
        self.ensure_history(&mut parts, now)?;
        let ids = distinct(&input.record_ids);
        for id in &ids {
            live(parts.session, id, RecordKind::HistoryEntry)?;
        }
        let targets: Vec<(Uuid, RecordKind)> = ids
            .iter()
            .map(|id| (*id, RecordKind::HistoryEntry))
            .collect();
        self.remove_rows(&mut parts, &targets, now)
    }

    /// Deletes every entry of `command` for `host` (or for no host): a row
    /// of `history_list_distinct` with all its repeats, also those recorded
    /// after the list was read (D64). Returns how many were deleted.
    pub fn history_delete_command(
        &self,
        host: Option<RecordRef>,
        command: &str,
    ) -> Result<usize, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_history(&mut parts, now)?;
        let targets: Vec<(Uuid, RecordKind)> = parts
            .session
            .index
            .of_kind(RecordKind::HistoryEntry)
            .filter(|record| {
                same_link(record.data(), host) && history_entry::command(record.data()) == command
            })
            .map(|record| (record.record_id, RecordKind::HistoryEntry))
            .collect();
        let count = targets.len();
        self.remove_rows(&mut parts, &targets, now)?;
        Ok(count)
    }

    pub(super) fn history_clear_impl(
        &self,
        input: HistoryClearInput,
    ) -> Result<HistoryClearResult, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_history(&mut parts, now)?;
        let mut targets: Vec<(Uuid, RecordKind)> = parts
            .session
            .index
            .of_kind(RecordKind::HistoryEntry)
            .filter(|record| same_link(record.data(), input.host))
            .map(|record| (record.record_id, RecordKind::HistoryEntry))
            .collect();
        targets.sort_by_key(|(id, _)| *id);
        if !targets.is_empty() {
            self.remove_rows(&mut parts, &targets, now)?;
        }
        tracing::info!(removed = targets.len(), "history cleared");
        Ok(HistoryClearResult {
            removed: u32::try_from(targets.len()).unwrap_or(u32::MAX),
        })
    }

    pub(super) fn connection_log_start_impl(
        &self,
        entry: ConnectionLogStart,
    ) -> Result<Uuid, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let started_at = format_timestamp(now);
        let data = connection_log::new_ssh(&connection_log::Start {
            started_at: &started_at,
            host: entry.host,
            host_label: entry.host_label.as_deref(),
            address: &entry.address,
            port: entry.port.max(1),
            username: entry.username.as_deref(),
            os: entry.os.as_ref(),
            device_id: parts.session.device_id,
        });
        let log_id = parts.ids.next(now);
        self.write_record(
            &mut parts,
            RecordKind::ConnectionLogEntry,
            log_id,
            data,
            now,
        )?;
        parts.connlog.started(log_id);
        tracing::debug!(log_id = %log_id, "connection log entry started");
        Ok(log_id)
    }

    pub(super) fn connection_log_end_impl(
        &self,
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    ) {
        let now = self.now();
        let end = PendingEnd {
            log_id,
            ended_at: format_timestamp(now),
            reason,
            exit_status,
        };
        let mut guard = self.inner();
        let inner = &mut *guard;
        if Self::require_session(inner).is_err() {
            inner.connlog.defer(end);
            tracing::debug!(log_id = %log_id, "connection log end held until unlock");
            return;
        }
        let Ok(mut parts) = parts(inner) else {
            return;
        };
        if let Err(error) = self.write_connlog_end(&mut parts, &end, now) {
            tracing::warn!(code = error.code(), "connection log end deferred");
            parts.connlog.defer(end);
        }
    }

    fn write_connlog_end(
        &self,
        parts: &mut Parts<'_>,
        end: &PendingEnd,
        now: SystemTime,
    ) -> Result<(), CoreError> {
        self.ensure_connlog(parts, now)?;
        let Some(record) = parts
            .session
            .index
            .get_kind(&end.log_id, RecordKind::ConnectionLogEntry)
        else {
            parts.connlog.ended(&end.log_id);
            return Ok(());
        };
        let mut data = record.data().clone();
        connection_log::set_end(
            &mut data,
            &end.ended_at,
            connection_log::reason_name(end.reason),
            end.exit_status,
        );
        self.write_record(parts, RecordKind::ConnectionLogEntry, end.log_id, data, now)?;
        parts.connlog.ended(&end.log_id);
        Ok(())
    }

    /// Retention on its own (UI 12.6, D68: at app start and every 24 hours):
    /// how many entries of this device were removed.
    pub(super) fn connection_log_prune_impl(&self) -> Result<usize, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let retention_days = retention_days(&guard.settings);
        let mut parts = parts(&mut guard)?;
        self.ensure_connlog(&mut parts, now)?;
        self.prune_connlog(&mut parts, retention_days, now)
    }

    pub(super) fn connection_log_list_impl(&self) -> Result<Vec<ConnectionLogView>, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let retention_days = retention_days(&guard.settings);
        let mut parts = parts(&mut guard)?;
        self.ensure_connlog(&mut parts, now)?;
        self.prune_connlog(&mut parts, retention_days, now)?;
        let cutoff = retention_cutoff(retention_days, now);
        let session = &*parts.session;
        let index = &session.index;
        let personal = session.ring.vault_id();
        let mut views: Vec<ConnectionLogView> = index
            .of_kind(RecordKind::ConnectionLogEntry)
            // Other devices' expired entries are hidden (their devices
            // remove them, data-model 13.3).
            .filter(|record| !expired(record.data(), &cutoff))
            .map(|record| {
                let data = record.data();
                let host = connection_log::host(data);
                let live = host
                    .filter(|link| link.vault_id == personal)
                    .and_then(|link| index.get_kind(&link.record_id, RecordKind::Host));
                let address = connection_log::address(data).to_owned();
                let title = match live {
                    Some(live) => host::title(live.data()).to_owned(),
                    None => connection_log::host_label(data)
                        .unwrap_or(&address)
                        .to_owned(),
                };
                ConnectionLogView {
                    record_id: record.record_id,
                    started_at: connection_log::started_at(data).to_owned(),
                    ended_at: connection_log::ended_at(data).map(str::to_owned),
                    end_reason: connection_log::end_reason(data).map(str::to_owned),
                    exit_status: connection_log::exit_status(data),
                    protocol: data
                        .get("protocol")
                        .and_then(Value::as_str)
                        .unwrap_or("ssh")
                        .to_owned(),
                    host,
                    title,
                    port: connection_log::port(data),
                    address,
                    username: connection_log::username(data).map(str::to_owned),
                    os: connection_log::os(data),
                    saved: connection_log::saved(data),
                    this_device: connection_log::device_id(data) == Some(session.device_id),
                    recorded: super::recordings::file_id_of(data).is_some(),
                }
            })
            .collect();
        views.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.record_id.cmp(&a.record_id))
        });
        Ok(views)
    }

    /// Retention (UI 12.6; data-model 13.3): this device's entries whose
    /// end (or start, while open) is older than `days` and that are not
    /// saved are removed.
    fn prune_connlog(
        &self,
        parts: &mut Parts<'_>,
        days: u64,
        now: SystemTime,
    ) -> Result<usize, CoreError> {
        let cutoff = retention_cutoff(days, now);
        let device = parts.session.device_id;
        let (expired, files): (Vec<(Uuid, RecordKind)>, Vec<Option<Uuid>>) = parts
            .session
            .index
            .of_kind(RecordKind::ConnectionLogEntry)
            .filter(|record| {
                connection_log::device_id(record.data()) == Some(device)
                    && !parts.connlog.is_active(&record.record_id)
                    && expired(record.data(), &cutoff)
            })
            .map(|record| {
                (
                    (record.record_id, RecordKind::ConnectionLogEntry),
                    super::recordings::file_id_of(record.data()),
                )
            })
            .unzip();
        if expired.is_empty() {
            return Ok(0);
        }
        tracing::info!(
            count = expired.len(),
            "expired connection log entries removed"
        );
        self.remove_rows_with(parts, &expired, now, false)?;
        // The entries first, their recordings second (storage-format 8.4).
        let files: Vec<Uuid> = files.into_iter().flatten().collect();
        super::recordings::delete_files(parts.store, &files);
        Ok(expired.len())
    }

    /// "Clear the log" (UI 12.6; data-model 13.3): every personal entry,
    /// saved or not, whatever device wrote it (an imported log too); the
    /// open sessions' entries as well (their end is then not written).
    pub(super) fn connection_log_clear_impl(&self) -> Result<usize, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_connlog(&mut parts, now)?;
        let (ids, files): (Vec<(Uuid, RecordKind)>, Vec<Option<Uuid>>) = parts
            .session
            .index
            .of_kind(RecordKind::ConnectionLogEntry)
            .map(|record| {
                (
                    (record.record_id, RecordKind::ConnectionLogEntry),
                    super::recordings::file_id_of(record.data()),
                )
            })
            .unzip();
        let count = ids.len();
        self.remove_rows(&mut parts, &ids, now)?;
        let files: Vec<Uuid> = files.into_iter().flatten().collect();
        super::recordings::delete_files(parts.store, &files);
        tracing::info!(count, "connection log cleared");
        Ok(count)
    }

    pub(super) fn connection_log_set_saved_impl(
        &self,
        log_id: Uuid,
        saved: bool,
    ) -> Result<(), CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_connlog(&mut parts, now)?;
        let record = parts
            .session
            .index
            .get_kind(&log_id, RecordKind::ConnectionLogEntry)
            .ok_or(CoreError::NotFound)?;
        let mut data = record.data().clone();
        data.insert("saved".into(), json!(saved));
        self.write_record(
            &mut parts,
            RecordKind::ConnectionLogEntry,
            log_id,
            data,
            now,
        )
    }

    pub(super) fn recent_connections_impl(&self) -> Result<Vec<RecentConnection>, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        self.ensure_connlog(&mut parts, now)?;
        let session = &*parts.session;
        let index = &session.index;
        let tree = index.tree();
        let personal = session.ring.vault_id();
        let vault_name = records::vault_view(session).name;
        let mut entries: Vec<&IndexedRecord> = index
            .of_kind(RecordKind::ConnectionLogEntry)
            .filter(|record| connection_log::is_ssh(record.data()))
            .collect();
        entries.sort_by(|a, b| {
            connection_log::started_at(b.data())
                .cmp(connection_log::started_at(a.data()))
                .then_with(|| b.record_id.cmp(&a.record_id))
        });
        let mut seen: HashSet<RecentKey> = HashSet::new();
        let mut rows = Vec::new();
        for record in entries {
            let data = record.data();
            let address = connection_log::address(data);
            let port = connection_log::port(data);
            let host_record = connection_log::host(data)
                .filter(|link| link.vault_id == personal)
                .and_then(|link| index.get_kind(&link.record_id, RecordKind::Host));
            // A removed host cannot be opened: its rows connect like a quick
            // connect to the address, port and username they recorded.
            let link = connection_log::host(data)
                .filter(|link| link.vault_id != personal || host_record.is_some());
            let group_key = match link {
                Some(link) => (Some((link.vault_id, link.record_id)), String::new(), 0),
                None => (None, address.to_lowercase(), port),
            };
            if !seen.insert(group_key) {
                continue;
            }
            let title = match host_record {
                Some(host_record) => host::title(host_record.data()).to_owned(),
                None => connection_log::host_label(data)
                    .unwrap_or(address)
                    .to_owned(),
            };
            let path = match host_record {
                Some(host_record) => {
                    let group = Index::host_group(&tree, host_record.data());
                    std::iter::once(vault_name.clone())
                        .chain(tree.host_path(group).iter().filter_map(|id| {
                            index
                                .get_kind(id, RecordKind::Group)
                                .map(|group| group::name(group.data()).to_owned())
                        }))
                        .collect()
                }
                None => Vec::new(),
            };
            let os = host_record
                .and_then(|host_record| host_os(host_record.data()))
                .or_else(|| connection_log::os(data));
            rows.push(RecentConnection {
                host: link,
                title,
                address: address.to_owned(),
                port,
                username: connection_log::username(data).map(str::to_owned),
                os,
                path,
                last_at: connection_log::started_at(data).to_owned(),
            });
            if rows.len() >= RECENT_LIMIT {
                break;
            }
        }
        Ok(rows)
    }
}

/// The History tab's "save as snippets" hint (stage 2b).
mod hint;

#[cfg(test)]
mod tests;
