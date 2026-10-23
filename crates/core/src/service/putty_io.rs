//! Import of PuTTY saved sessions (SPEC "Import"; UI 2.5; run 8 decision
//! D140): the preview and the import, over the reader of
//! `crate::import::putty`. The sessions come through a
//! [`SessionSource`]: the Windows registry in `shellpp-desktop`, a fake in
//! tests.
//!
//! Mapping of each session (`Default Settings` is a template and never a
//! host):
//! - the session name is the label; `HostName` (a `user@` in it gives the
//!   user name) → address; `PortNumber`, `UserName` → `ssh.port`,
//!   `ssh.username`;
//! - `Protocol` `ssh` → an SSH host, `telnet` → a Telnet host
//!   (`primary_protocol = telnet`, `telnet.port`, `telnet.username`);
//!   `raw`, `rlogin`, `serial`, `supdup` and `bare` are not imported;
//! - `PublicKeyFile` → the `.ppk` (or OpenSSH) key file is imported into
//!   the Keychain (an encrypted one with the passphrase given in the
//!   preview) and linked as `ssh.key_id`;
//! - `ProxyMethod` SOCKS 5 and HTTP → `ssh.proxy` with its user name and
//!   password; SOCKS 4, Telnet and SSH proxies are not imported, and a
//!   local proxy command is neither run nor stored (counted in the
//!   summary): such a host is refused rather than connected directly;
//! - `PortForwardings` → forwarding rules of the host that start when it
//!   connects, except one that listens beyond loopback (PuTTY's "Local
//!   ports accept connections from other hosts" included), which the user
//!   turns on;
//! - `AgentFwd` → agent forwarding of the system agent (Pageant, D60);
//!   `PingIntervalSecs` → `ssh.keepalive_s` (when not 0); `Environment` →
//!   `ssh.env`; `BackspaceIsDelete 0` → `backspace = ctrl_h`;
//!   `LineCodePage` → `charset`;
//! - X11 forwarding, a remote command, a detached certificate and a user
//!   name taken from the local user are listed in the host's notes.
//!
//! Idempotent: a host carries `import_key` = `putty:<session name>`
//! (docs/data-model.md, section 18.4), matched case-insensitively as the
//! registry matches key names; such a host is "already exists" and never
//! overwritten. Keys match by fingerprint.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use secrecy::SecretString;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::Core;
use super::dto::{
    ImportStatus, PuttyImportInput, SshConfigHostPreview, SshConfigImportSummary,
    SshConfigKeyPreview, SshConfigPreview,
};
use super::records::{Plan, parts};
use super::ssh_config_io::{bind_spec, destination, host_notes, key_file_status};
use crate::error::CoreError;
use crate::import::putty::{self, ForwardKind, Session, SessionSource};
use crate::model::host::{self, HostRules};
use crate::model::types::RecordKind;

/// The heading of an imported host's notes.
const NOTES_HEADING: &str = "Imported from PuTTY.";
/// The longest keepalive (`ssh.keepalive_s`).
const MAX_KEEPALIVE_S: u32 = 3600;

/// PuTTY's proxy types (`ProxyMethod`).
const PROXY_NONE: u32 = 0;
const PROXY_SOCKS4: u32 = 1;
const PROXY_SOCKS5: u32 = 2;
const PROXY_HTTP: u32 = 3;
const PROXY_TELNET: u32 = 4;
const PROXY_CMD: u32 = 5;

/// The import key of a host from a PuTTY session.
pub fn import_key(name: &str) -> String {
    format!("putty:{name}")
}

/// What the import does with one session.
struct SessionPlan {
    name: String,
    address: String,
    /// The SSH or Telnet port.
    port: Option<u16>,
    username: Option<String>,
    /// Top-level members other than `label`, `address`, `ssh` and `notes`.
    top: Map<String, Value>,
    /// `ssh` members other than `key_id`.
    ssh: Map<String, Value>,
    proxy_password: Option<SecretString>,
    key_file: Option<PathBuf>,
    /// Forwarding rule data (`type`, bind and destination), in order.
    forwards: Vec<Map<String, Value>>,
    notes: Vec<String>,
    error: Option<String>,
    /// The session's proxy runs a local command (never run or stored).
    command: bool,
}

impl SessionPlan {
    /// The host data to build (without `key_id`), and its secrets.
    fn data(
        &self,
        key_id: Option<Uuid>,
    ) -> (Map<String, Value>, BTreeMap<String, Option<SecretString>>) {
        let mut data = self.top.clone();
        data.insert("label".into(), json!(self.name));
        data.insert("address".into(), json!(self.address));
        let mut ssh = self.ssh.clone();
        if let Some(key_id) = key_id {
            ssh.insert("key_id".into(), json!(key_id.to_string()));
        }
        data.insert("ssh".into(), Value::Object(ssh));
        if let Some(notes) = host_notes(NOTES_HEADING, &self.notes) {
            data.insert("notes".into(), json!(notes));
        }
        let mut secrets = BTreeMap::new();
        if let Some(password) = &self.proxy_password {
            secrets.insert("ssh.proxy.password".to_owned(), Some(password.clone()));
        }
        (data, secrets)
    }
}

/// The proxy type of a session: `ProxyMethod`, or the `ProxyType` and
/// `ProxySOCKSVersion` of PuTTY before 0.71.
fn proxy_method(session: &Session) -> u32 {
    if let Some(method) = session.number("ProxyMethod") {
        return method;
    }
    match session.number("ProxyType").unwrap_or(0) {
        0 => PROXY_NONE,
        1 => PROXY_HTTP,
        2 if session.number("ProxySOCKSVersion") == Some(4) => PROXY_SOCKS4,
        2 => PROXY_SOCKS5,
        3 => PROXY_TELNET,
        _ => PROXY_CMD,
    }
}

/// An `ssh.proxy` object and its password.
type Proxy = (Map<String, Value>, Option<SecretString>);

/// The `ssh.proxy` of a session, `Ok(None)` without one, or why it is not
/// imported.
fn proxy_of(
    session: &Session,
    plan_command: &mut bool,
) -> Result<Option<Proxy>, String> {
    let kind = match proxy_method(session) {
        PROXY_NONE => return Ok(None),
        PROXY_SOCKS5 => "socks5",
        PROXY_HTTP => "http",
        PROXY_SOCKS4 => return Err("a SOCKS 4 proxy is not supported".into()),
        PROXY_TELNET => return Err("a Telnet proxy is not supported".into()),
        PROXY_CMD => {
            *plan_command = true;
            return Err("the proxy is a local command, which is not run or imported".into());
        }
        _ => return Err("an SSH proxy is not supported".into()),
    };
    let Some(address) = session.nonempty("ProxyHost") else {
        return Err("the proxy has no host name".into());
    };
    let port = match session.number("ProxyPort").unwrap_or(80) {
        port @ 1..=65535 => port,
        _ => return Err("the proxy port is not valid".into()),
    };
    let mut proxy = Map::new();
    proxy.insert("type".into(), json!(kind));
    proxy.insert("address".into(), json!(address));
    proxy.insert("port".into(), json!(port));
    if let Some(user) = session.nonempty("ProxyUsername") {
        proxy.insert("username".into(), json!(user));
    }
    let password = session
        .text("ProxyPassword")
        .filter(|password| !password.is_empty())
        .map(|password| SecretString::from(password.to_owned()));
    Ok(Some((proxy, password)))
}

/// The forwarding rules of `PortForwardings`, and notes for the entries the
/// rules cannot hold.
fn forwards_of(session: &Session, notes: &mut Vec<String>) -> Vec<Map<String, Value>> {
    let Some(text) = session.nonempty("PortForwardings") else {
        return Vec::new();
    };
    let (found, bad) = putty::forwards(text);
    for raw in bad {
        notes.push(format!(
            "Port forwarding {raw} is not imported: this form is not supported"
        ));
    }
    let local_all = session.number("LocalPortAcceptAll") == Some(1);
    let remote_all = session.number("RemotePortAcceptAll") == Some(1);
    let mut rules = Vec::new();
    for forward in found {
        let (kind, accept_all) = match forward.kind {
            ForwardKind::Local => ("local", local_all),
            ForwardKind::Remote => ("remote", remote_all),
            ForwardKind::Dynamic => ("dynamic", local_all),
        };
        let bind = bind_spec(&forward.source).map(|(address, port)| {
            // A bare port listens on loopback, or on every interface with
            // PuTTY's "accept connections from other hosts".
            let bare = !forward.source.contains([':', '/']);
            if bare && accept_all {
                ("0.0.0.0".to_owned(), port)
            } else {
                (address, port)
            }
        });
        let dest = match &forward.destination {
            Some(raw) => destination(raw).map(Some),
            None => Some(None),
        };
        let (Some((bind_address, bind_port)), Some(dest)) = (bind, dest) else {
            notes.push(format!(
                "Port forwarding {} is not imported: this form is not supported",
                forward.raw
            ));
            continue;
        };
        if let Some(family) = forward.family {
            notes.push(format!(
                "Port forwarding {}: PuTTY limits it to IPv{family}; it is imported without the limit",
                forward.raw
            ));
        }
        // A rule that listens beyond loopback needs the user's own choice
        // (security.md 11.3): an imported session does not open such a
        // port on its own.
        let wide = crate::model::snippet::is_wide_bind(&bind_address);
        if wide {
            notes.push(format!(
                "Port forwarding {} listens on all interfaces: it is imported without autostart",
                forward.raw
            ));
        }
        let mut rule = Map::new();
        rule.insert("type".into(), json!(kind));
        rule.insert("bind_address".into(), json!(bind_address));
        rule.insert("bind_port".into(), json!(bind_port));
        if let Some((host, port)) = dest {
            rule.insert("dest_address".into(), json!(host));
            rule.insert("dest_port".into(), json!(port));
        }
        rule.insert("autostart_on_connect".into(), json!(!wide));
        rules.push(rule);
    }
    rules
}

/// `ssh.env` from `Environment` (names unique, at most 64).
fn env_of(session: &Session, notes: &mut Vec<String>) -> Vec<Value> {
    let Some(text) = session.nonempty("Environment") else {
        return Vec::new();
    };
    let mut env = Vec::new();
    let mut names = HashSet::new();
    for (name, value) in putty::map_entries(text) {
        let value = value.unwrap_or_default();
        if name.is_empty()
            || value.len() > host::MAX_ENV_VALUE_BYTES
            || env.len() >= host::MAX_ENV_VARS
        {
            notes.push(format!("Environment variable {name} is not imported"));
            continue;
        }
        if names.insert(name.clone()) {
            env.push(json!({"name": name, "value": value}));
        }
    }
    env
}

fn plan_session(session: &Session) -> SessionPlan {
    let name = session.name();
    let mut notes = Vec::new();
    let mut error = None;
    let mut command = false;
    let mut top = Map::new();
    let mut ssh = Map::new();

    let protocol = session
        .nonempty("Protocol")
        .unwrap_or("ssh")
        .to_ascii_lowercase();
    let telnet = match protocol.as_str() {
        "ssh" => false,
        "telnet" => true,
        other => {
            error = Some(format!("the {other} protocol is not supported"));
            false
        }
    };

    // `user@host` in the host name field, as PuTTY accepts it.
    let raw_host = session.nonempty("HostName").unwrap_or_default();
    let (host_user, address) = match raw_host.rsplit_once('@') {
        Some((user, host)) => (Some(user.to_owned()), host),
        None => (None, raw_host),
    };
    let address = address
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(address)
        .to_owned();
    if address.is_empty() && error.is_none() {
        error = Some("the session has no host name".into());
    }
    let username = session
        .nonempty("UserName")
        .map(str::to_owned)
        .or(host_user)
        .filter(|user| !user.is_empty());
    if username.is_none() && session.number("UserNameFromEnvironment") == Some(1) {
        notes.push("The user name comes from the local user in PuTTY and is not imported".into());
    }
    let port = match session.number("PortNumber") {
        Some(port @ 1..=65535) => u16::try_from(port).ok(),
        Some(0) | None => None,
        Some(_) => {
            error.get_or_insert_with(|| "the port is not valid".into());
            None
        }
    };

    if telnet {
        top.insert("primary_protocol".into(), json!("telnet"));
        let mut telnet = Map::new();
        telnet.insert("port".into(), json!(port.unwrap_or(23)));
        if let Some(user) = &username {
            telnet.insert("username".into(), json!(user));
        }
        top.insert("telnet".into(), Value::Object(telnet));
    } else {
        if let Some(port) = port {
            ssh.insert("port".into(), json!(port));
        }
        if let Some(user) = &username {
            ssh.insert("username".into(), json!(user));
        }
    }

    let mut proxy_password = None;
    match proxy_of(session, &mut command) {
        Ok(Some((proxy, password))) => {
            ssh.insert("proxy".into(), Value::Object(proxy));
            proxy_password = password;
            if session.nonempty("ProxyExcludeList").is_some() {
                notes.push(
                    "ProxyExcludeList is not imported: the proxy is used for this host".into(),
                );
            }
        }
        Ok(None) => {}
        Err(reason) => {
            error.get_or_insert(reason);
        }
    }

    let key_file = if telnet {
        None
    } else {
        session.nonempty("PublicKeyFile").map(PathBuf::from)
    };
    if !telnet {
        if session.number("AgentFwd") == Some(1) {
            ssh.insert("agent_forwarding".into(), json!(true));
            // PuTTY forwards Pageant: the system agent (D60).
            ssh.insert("agent_forward_system".into(), json!(true));
        }
        let keepalive = session
            .number("PingIntervalSecs")
            .or_else(|| {
                session
                    .number("PingInterval")
                    .map(|min| min.saturating_mul(60))
            })
            .unwrap_or(0);
        if keepalive > 0 {
            if keepalive > MAX_KEEPALIVE_S {
                notes.push("A keepalive over 3600 s is imported as 3600".into());
            }
            ssh.insert("keepalive_s".into(), json!(keepalive.min(MAX_KEEPALIVE_S)));
        }
        let env = env_of(session, &mut notes);
        if !env.is_empty() {
            ssh.insert("env".into(), Value::Array(env));
        }
        if session.number("X11Forward") == Some(1) {
            notes.push("X11 forwarding is not imported".into());
        }
        if session.nonempty("RemoteCommand").is_some() {
            notes.push("The remote command is not imported".into());
        }
        if session.nonempty("DetachedCertificate").is_some() {
            notes.push("The detached certificate is not imported yet".into());
        }
    }
    let forwards = if telnet {
        Vec::new()
    } else {
        forwards_of(session, &mut notes)
    };

    if session.number("BackspaceIsDelete") == Some(0) {
        top.insert("backspace".into(), json!("ctrl_h"));
    }
    if let Some(code_page) = session.text("LineCodePage") {
        match putty::charset_label(code_page) {
            Some(label) if label == "utf-8" => {}
            Some(label) => {
                top.insert("charset".into(), json!(label));
            }
            None => notes.push(format!(
                "The character set {} is not supported; UTF-8 is used",
                code_page.trim()
            )),
        }
    }

    SessionPlan {
        name,
        address,
        port,
        username,
        top,
        ssh,
        proxy_password,
        key_file,
        forwards,
        notes,
        error,
        command,
    }
}

/// Plans every session but the template, in the source's order, each name
/// once (case-insensitively).
fn plan_all(sessions: &[Session]) -> Vec<SessionPlan> {
    let mut seen = HashSet::new();
    let mut plans = Vec::new();
    for session in sessions {
        let name = session.name();
        if name == putty::DEFAULT_SETTINGS || !seen.insert(name.to_lowercase()) {
            continue;
        }
        let mut plan = plan_session(session);
        // A host the rules refuse (an invalid address, a label too long)
        // is shown as not imported in the preview already.
        if plan.error.is_none() {
            let (data, secrets) = plan.data(None);
            let rules = HostRules {
                record_id: None,
                personal_vault: true,
            };
            if let Err(errors) = host::build(&data, None, &secrets, &rules) {
                let fields: Vec<String> = errors.into_iter().map(|error| error.field).collect();
                plan.error = Some(format!("invalid {}", fields.join(", ")));
            }
        }
        plans.push(plan);
    }
    plans
}

impl Core {
    /// The sessions of `source`; `Unavailable` when it cannot be read.
    fn putty_sessions(source: &dyn SessionSource) -> Result<putty::Sessions, CoreError> {
        let mut sessions = source.sessions().map_err(|error| {
            tracing::info!(kind = ?error.kind(), "putty sessions not read");
            CoreError::Unavailable
        })?;
        sessions.sessions.truncate(putty::MAX_SESSIONS);
        Ok(sessions)
    }

    /// The live hosts of earlier PuTTY imports: session name (lowercase) → id.
    fn imported_sessions(&self) -> Result<HashMap<String, Uuid>, CoreError> {
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        Ok(parts
            .session
            .index
            .of_kind(RecordKind::Host)
            .filter_map(|record| {
                record
                    .data()
                    .get("import_key")
                    .and_then(Value::as_str)
                    .and_then(|key| key.strip_prefix("putty:"))
                    .map(|name| (name.to_lowercase(), record.record_id))
            })
            .collect())
    }

    pub(super) fn putty_preview_impl(
        &self,
        source: &dyn SessionSource,
    ) -> Result<SshConfigPreview, CoreError> {
        {
            let inner = self.inner();
            Self::require_session(&inner)?;
        }
        let sessions = Self::putty_sessions(source)?;
        let plans = plan_all(&sessions.sessions);
        let imported = self.imported_sessions()?;
        let known: HashSet<String> = self
            .keys_list()?
            .into_iter()
            .filter_map(|key| key.fingerprint)
            .collect();
        let mut keys: Vec<SshConfigKeyPreview> = Vec::new();
        let mut seen_keys = HashSet::new();
        let mut hosts = Vec::new();
        for plan in plans {
            let status = if plan.error.is_some() {
                ImportStatus::Error
            } else if imported.contains_key(&plan.name.to_lowercase()) {
                ImportStatus::Exists
            } else {
                ImportStatus::New
            };
            if let Some(path) = &plan.key_file
                && seen_keys.insert(path.clone())
            {
                keys.push(SshConfigKeyPreview {
                    path: path.display().to_string(),
                    status: key_file_status(path, &known),
                });
            }
            hosts.push(SshConfigHostPreview {
                alias: plan.name,
                address: plan.address,
                port: plan.port,
                username: plan.username,
                status,
                reason: plan.error,
                notes: plan.notes,
                key_files: plan
                    .key_file
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                jump: Vec::new(),
            });
        }
        Ok(SshConfigPreview {
            hosts,
            keys,
            problems: sessions
                .unreadable
                .iter()
                .map(|key| {
                    format!(
                        "{}: the session could not be read",
                        putty::session_name(key)
                    )
                })
                .collect(),
            templates: 0,
        })
    }

    pub(super) fn putty_import_impl(
        &self,
        source: &dyn SessionSource,
        input: PuttyImportInput,
    ) -> Result<SshConfigImportSummary, CoreError> {
        let vault_id = {
            let inner = self.inner();
            Self::require_session(&inner)?.ring.vault_id()
        };
        let sessions = Self::putty_sessions(source)?;
        let plans = plan_all(&sessions.sessions);
        let imported = self.imported_sessions()?;
        let chosen: Option<HashSet<String>> = input
            .sessions
            .as_ref()
            .map(|names| names.iter().map(|name| name.to_lowercase()).collect());
        let mut summary = SshConfigImportSummary::default();
        let mut selected: Vec<SessionPlan> = Vec::new();
        for plan in plans {
            let name = plan.name.to_lowercase();
            if chosen
                .as_ref()
                .is_some_and(|chosen| !chosen.contains(&name))
            {
                continue;
            }
            if plan.command {
                summary.commands_skipped += 1;
            }
            if imported.contains_key(&name) {
                summary.skipped += 1;
                continue;
            }
            if let Some(reason) = &plan.error {
                summary.errors.push((plan.name.clone(), reason.clone()));
                continue;
            }
            selected.push(plan);
        }

        let key_ids = self.import_key_files(
            vault_id,
            selected.iter().filter_map(|plan| plan.key_file.as_ref()),
            &input.passphrases,
            input.save_passphrases,
            &mut summary,
        );

        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let mut plan_writes = Plan::default();
        let mut writes = 0usize;
        for plan in selected {
            let id = parts.ids.next(now);
            let key_id = plan
                .key_file
                .as_ref()
                .and_then(|path| key_ids.get(path))
                .copied();
            let (data, secrets) = plan.data(key_id);
            let rules = HostRules {
                record_id: Some(id),
                personal_vault: true,
            };
            match host::build(&data, None, &secrets, &rules) {
                Ok(mut built) => {
                    built.insert("import_key".into(), json!(import_key(&plan.name)));
                    plan_writes.set(RecordKind::Host, id, built);
                    writes += 1;
                    for (index, rule) in plan.forwards.iter().enumerate() {
                        let mut rule = rule.clone();
                        rule.insert("host_id".into(), json!(id.to_string()));
                        if let Ok(mut built) = crate::model::snippet::build_rule(&rule, None) {
                            built.insert(
                                "import_key".into(),
                                json!(format!("{}:{index}", import_key(&plan.name))),
                            );
                            plan_writes.set(RecordKind::ForwardingRule, parts.ids.next(now), built);
                            summary.rules_added += 1;
                        }
                    }
                }
                Err(errors) => {
                    let fields: Vec<String> = errors.into_iter().map(|error| error.field).collect();
                    summary
                        .errors
                        .push((plan.name, format!("invalid {}", fields.join(", "))));
                }
            }
        }
        if writes > 0 {
            let prepared = plan_writes.prepare(&mut parts, now)?;
            self.commit_records(&mut parts, prepared, Vec::new(), Vec::new())?;
        }
        summary.added = writes;
        tracing::info!(
            added = summary.added,
            skipped = summary.skipped,
            errors = summary.errors.len(),
            keys_added = summary.keys_added,
            "putty sessions imported"
        );
        Ok(summary)
    }
}
