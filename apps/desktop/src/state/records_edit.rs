//! Editing hosts and groups from the details panel (UI 1.3, 3.1-3.4;
//! docs/data-model.md, sections 7 and 8): the form values ([`HostDraft`],
//! [`GroupDraft`]), their validation, the saving rule and inheritance.
//!
//! Saving rule (UI 1.3, 3.2):
//! - a new record stays a draft until its required field is valid (a host's
//!   address, a group's name); nothing is written before that;
//! - after that every valid change is written; an invalid member is reported
//!   with its reason and is not written: the stored (last valid) value of that
//!   member stays in effect while the other members are saved.
//!
//! [`AppState::save_host_draft`] and [`AppState::save_group_draft`] implement
//! the rule: client-side checks first, then the core's own validation; a
//! member the core refuses is put back to its stored value and the save is
//! retried once.
//!
//! Inheritance ([`AppState::inherited_ssh`]): `port`, `username`,
//! `identity_id`, `key_id` and `jump_chain` come from the nearest group up the
//! parent chain that sets them (data-model 8.2, 8.3); the panels show them as
//! the placeholder "from group X".

use std::collections::{BTreeMap, BTreeSet};

use gpui::{AppContext as _, Context, Task};
use secrecy::SecretString;
use serde_json::{Map, Value, json};
pub use shellpp_core::keys::generate::KeyKind;
pub use shellpp_core::model::identity::PasswordChange;
use shellpp_core::model::{fields, text};
use shellpp_core::service::dto::{
    KeyGenerateInput, KeyImportInput, KeyImportResult, KeyView, RecordRef, RecordSaveInput,
    RecordType, RecordView, SecretCopyTarget, SecretFieldRef, TagColor,
};
pub use shellpp_core::service::keychain::IdentityView;
use shellpp_core::service::keychain::{IdentitySaveInput, KeyCertificateInput, KeyLabelInput};
use shellpp_core::{CoreError, FieldError, ValidationReason};
use uuid::Uuid;

use super::{AppState, GroupItem, HostItem, SelectMode};

/// Nesting limit of the lookup chain (data-model 8.2).
const MAX_CHAIN: usize = 32;
/// Maximum jump hosts (data-model 7.3).
pub const MAX_JUMP_HOSTS: usize = 8;
/// Maximum tags on a host (data-model 3.5).
pub const MAX_TAGS: usize = 32;

// -- Inheritance -----------------------------------------------------------------

/// A value inherited from a group, with the group that supplies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InheritedFrom<T> {
    pub value: T,
    pub group_id: Uuid,
}

/// The SSH values a host or group placed inside a group would inherit
/// (data-model 8.1). A field is `None` when no group up the chain sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InheritedSsh {
    pub port: Option<InheritedFrom<u16>>,
    pub username: Option<InheritedFrom<String>>,
    pub identity_id: Option<InheritedFrom<Uuid>>,
    pub key_id: Option<InheritedFrom<Uuid>>,
    /// `[]` at a level means "no jump hosts" and stops the lookup.
    pub jump_chain: Option<InheritedFrom<Vec<Uuid>>>,
}

fn uuid_member(ssh: &Map<String, Value>, key: &str) -> Option<Uuid> {
    ssh.get(key)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

impl InheritedSsh {
    /// Fills the fields still empty from one level's `ssh` block; the
    /// username and the key also from the identity the level names
    /// (`idf`, data-model 8.3), as a connection resolves them.
    fn absorb(&mut self, ssh: &Map<String, Value>, group_id: Uuid, idf: Option<&IdentityView>) {
        if self.port.is_none()
            && let Some(port) = ssh
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|port| u16::try_from(port).ok())
        {
            self.port = Some(InheritedFrom {
                value: port,
                group_id,
            });
        }
        if self.username.is_none()
            && let Some(username) = ssh
                .get("username")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    idf.and_then(|identity| identity.username.as_deref())
                        .filter(|value| !value.is_empty())
                })
        {
            self.username = Some(InheritedFrom {
                value: username.to_owned(),
                group_id,
            });
        }
        if self.identity_id.is_none()
            && let Some(id) = uuid_member(ssh, "identity_id")
        {
            self.identity_id = Some(InheritedFrom {
                value: id,
                group_id,
            });
        }
        if self.key_id.is_none()
            && let Some(id) =
                uuid_member(ssh, "key_id").or_else(|| idf.and_then(|identity| identity.key_id))
        {
            self.key_id = Some(InheritedFrom {
                value: id,
                group_id,
            });
        }
        if self.jump_chain.is_none()
            && let Some(chain) = ssh.get("jump_chain").and_then(Value::as_array)
        {
            let value = chain
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect();
            self.jump_chain = Some(InheritedFrom { value, group_id });
        }
    }
}

// -- Form values -------------------------------------------------------------------

/// What the Backspace key sends (UI 3.1; data-model 7.2 `backspace`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackspaceMode {
    #[default]
    Default,
    /// `^?` (DEL).
    Del,
    /// `^H`.
    CtrlH,
}

impl BackspaceMode {
    pub const ALL: [Self; 3] = [Self::Default, Self::Del, Self::CtrlH];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Del => "del",
            Self::CtrlH => "ctrl_h",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "del" => Self::Del,
            "ctrl_h" => Self::CtrlH,
            _ => Self::Default,
        }
    }
}

/// "As in settings" / On / Off (`record_history`, `record_output`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverrideMode {
    /// As in settings.
    #[default]
    Default,
    On,
    Off,
}

impl OverrideMode {
    pub const ALL: [Self; 3] = [Self::Default, Self::On, Self::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "on" => Self::On,
            "off" => Self::Off,
            _ => Self::Default,
        }
    }
}

/// Proxy type (data-model 7.5); `None` means no proxy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyKind {
    #[default]
    None,
    Http,
    Socks5,
}

impl ProxyKind {
    pub const ALL: [Self; 3] = [Self::None, Self::Http, Self::Socks5];

    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Http => Some("http"),
            Self::Socks5 => Some("socks5"),
        }
    }

    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("http") => Self::Http,
            Some("socks5") => Self::Socks5,
            _ => Self::None,
        }
    }
}

/// One environment variable row (data-model 3.5 `env_var`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// The Telnet protocol block of a host (UI 3.1 "Add Telnet"; data-model 7.4):
/// port (23 when empty) and the login name, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelnetDraft {
    pub port: String,
    pub username: String,
}

/// Default Telnet port (data-model 8.5).
pub const TELNET_DEFAULT_PORT: u16 = 23;

/// Charset labels offered by the Charset select (WHATWG Encoding labels,
/// stored in lowercase; data-model 7.2). `utf-8` is the default.
pub const CHARSETS: &[&str] = &[
    "utf-8",
    "iso-8859-1",
    "iso-8859-2",
    "iso-8859-5",
    "iso-8859-15",
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "koi8-r",
    "koi8-u",
    "ibm866",
    "gbk",
    "gb18030",
    "big5",
    "euc-jp",
    "shift_jis",
    "euc-kr",
];

/// The built-in terminal themes a host can name (data-model 3.5 `theme_ref`);
/// Graphite is the default (data-model 8.5).
pub const TERMINAL_THEMES: &[&str] = shellpp_core::model::fields::BUILTIN_THEMES;

/// What the password field asks to do with `ssh.password`.
#[derive(Clone, Debug, Default)]
pub enum SecretChange {
    /// Keep the stored value (the field was not edited).
    #[default]
    Keep,
    /// Store this value.
    Set(SecretString),
    /// Remove the stored value.
    Clear,
}

/// The secret members a host save changes (`ssh.password`,
/// `ssh.proxy.password`).
#[derive(Clone, Debug, Default)]
pub struct HostSecrets {
    pub password: SecretChange,
    pub proxy_password: SecretChange,
}

/// The values of the host panel form (UI 3.1), as typed. Text fields keep the
/// raw text so an invalid value can be reported; [`HostDraft::apply`] writes
/// them into a data object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostDraft {
    pub address: String,
    pub label: String,
    pub group_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub color: TagColor,
    pub backspace: BackspaceMode,
    pub notes: String,
    pub port: String,
    pub username: String,
    /// Linked credentials (UI 3.1 "+"): kept as stored, unlinked by the panel.
    pub identity_id: Option<Uuid>,
    pub key_id: Option<Uuid>,
    pub certificate_key_id: Option<Uuid>,
    pub fido2_key_id: Option<Uuid>,
    pub agent_forwarding: bool,
    /// Agent Forwarding also offers the system agent's keys (D60).
    pub agent_forward_system: bool,
    /// Agent Forwarding asks before every signature (D60).
    pub agent_confirm: bool,
    /// The startup snippet (`ssh.startup_snippet_id`), kept as stored.
    pub startup_snippet_id: Option<Uuid>,
    /// `None`: inherit from the group (or none).
    pub jump_chain: Option<Vec<Uuid>>,
    pub proxy_kind: ProxyKind,
    pub proxy_address: String,
    pub proxy_port: String,
    pub proxy_username: String,
    pub env: Vec<EnvVar>,
    /// A charset label; empty means the default (`utf-8`).
    pub charset: String,
    pub keepalive: String,
    pub record_history: OverrideMode,
    pub record_output: OverrideMode,
    /// `ssh.mosh` (lead decision 2: shown, only the value is saved).
    pub mosh: bool,
    /// A built-in theme slug; `None`: the global default (Graphite).
    pub terminal_theme: Option<String>,
    /// The Telnet block; `None` until Add Telnet.
    pub telnet: Option<TelnetDraft>,
}

fn str_of<'a>(map: &'a Map<String, Value>, key: &str) -> &'a str {
    map.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn int_text(map: &Map<String, Value>, key: &str) -> String {
    map.get(key)
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn uuid_list(value: Option<&Value>) -> Option<Vec<Uuid>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect()
    })
}

fn color_of(data: &Map<String, Value>) -> TagColor {
    data.get("color")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Sets `key` to the trimmed text, or removes it when the text is empty.
fn set_text(map: &mut Map<String, Value>, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_owned(), Value::from(value));
    }
}

/// Sets `key` to the number in `raw`, removes it when `raw` is empty, and
/// leaves an unparsable value as text (the core reports it).
fn set_int(map: &mut Map<String, Value>, key: &str, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty() {
        map.remove(key);
    } else if let Ok(value) = raw.parse::<u64>() {
        map.insert(key.to_owned(), json!(value));
    } else {
        map.insert(key.to_owned(), Value::from(raw));
    }
}

fn object_mut<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let entry = map
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry.as_object_mut().expect("object member")
}

/// Field errors by dotted path, the way the core names them (`address`,
/// `ssh.port`, `tags.3`, `ssh.env.2.name`).
pub type FieldErrors = BTreeMap<String, ValidationReason>;

/// The member an error path belongs to, for putting the stored value back:
/// the path up to its first list index (`ssh.env.2.name` -> `ssh.env`,
/// `tags.3` -> `tags`), and `ssh.proxy.*` -> `ssh.proxy`.
pub fn member_of(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('.') {
        if part.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        parts.push(part);
        if parts.as_slice() == ["ssh", "proxy"] {
            break;
        }
    }
    parts.join(".")
}

/// Copies the value at a dotted member path from `base` into `data`
/// (removing it when `base` has none).
fn restore_member(data: &mut Map<String, Value>, base: &Map<String, Value>, member: &str) {
    match member.split_once('.') {
        None => match base.get(member) {
            Some(value) => {
                data.insert(member.to_owned(), value.clone());
            }
            None => {
                data.remove(member);
            }
        },
        Some((head, rest)) => {
            let empty = Map::new();
            let base_inner = base.get(head).and_then(Value::as_object).unwrap_or(&empty);
            let inner = object_mut(data, head);
            restore_member(inner, base_inner, rest);
        }
    }
}

/// Client-side check of a port field (`int[1..65535]`, empty allowed).
pub fn check_port(raw: &str) -> Result<(), ValidationReason> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(());
    }
    match raw.parse::<u64>() {
        Ok(value) if (1..=65535).contains(&value) => Ok(()),
        Ok(_) => Err(ValidationReason::OutOfRange),
        Err(_) if raw.chars().all(|c| c.is_ascii_digit()) => Err(ValidationReason::OutOfRange),
        Err(_) => Err(ValidationReason::Invalid),
    }
}

/// Client-side check of the keepalive field (`int[0..3600]` seconds).
pub fn check_keepalive(raw: &str) -> Result<(), ValidationReason> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(());
    }
    match raw.parse::<u64>() {
        Ok(value) if value <= 3600 => Ok(()),
        Ok(_) => Err(ValidationReason::OutOfRange),
        Err(_) if raw.chars().all(|c| c.is_ascii_digit()) => Err(ValidationReason::OutOfRange),
        Err(_) => Err(ValidationReason::Invalid),
    }
}

/// A host address as the core accepts it (data-model 7.2). `Ok(None)`: empty.
pub fn check_address(raw: &str) -> Result<Option<String>, ValidationReason> {
    fields::address(raw)
}

/// Normalizes a tag typed in the Tags field (`string(64)`, no commas).
pub fn normalize_tag(raw: &str) -> Result<String, ValidationReason> {
    text::tag(raw)
}

/// Whether `tags` already holds `tag` (case-insensitive, data-model 3.5).
pub fn has_tag(tags: &[String], tag: &str) -> bool {
    let folded = text::fold(tag);
    tags.iter().any(|existing| text::fold(existing) == folded)
}

fn env_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    name.len() <= 256 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A charset label as the core accepts it (data-model 7.2): up to 64
/// characters of `a-z 0-9 . _ : -` after lowercasing.
fn charset_ok(raw: &str) -> bool {
    let label = raw.trim().to_lowercase();
    label.chars().count() <= 64
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:-".contains(&b))
}

impl HostDraft {
    /// The form of a stored host.
    pub fn from_host(host: &HostItem) -> Self {
        Self::from_data(&host.data_map(), host.group_id)
    }

    /// The form of a data object; `group_id` is the resolved group.
    pub fn from_data(data: &Map<String, Value>, group_id: Option<Uuid>) -> Self {
        let empty = Map::new();
        let ssh = data.get("ssh").and_then(Value::as_object).unwrap_or(&empty);
        let proxy = ssh.get("proxy").and_then(Value::as_object);
        let tags = data
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let env = ssh
            .get("env")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|item| EnvVar {
                        name: str_of(item, "name").to_owned(),
                        value: str_of(item, "value").to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            address: str_of(data, "address").to_owned(),
            label: str_of(data, "label").to_owned(),
            group_id,
            tags,
            color: color_of(data),
            backspace: BackspaceMode::parse(str_of(data, "backspace")),
            notes: str_of(data, "notes").to_owned(),
            port: int_text(ssh, "port"),
            username: str_of(ssh, "username").to_owned(),
            identity_id: uuid_member(ssh, "identity_id"),
            key_id: uuid_member(ssh, "key_id"),
            certificate_key_id: uuid_member(ssh, "certificate_key_id"),
            fido2_key_id: uuid_member(ssh, "fido2_key_id"),
            agent_forwarding: ssh
                .get("agent_forwarding")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            agent_forward_system: ssh
                .get("agent_forward_system")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            agent_confirm: ssh
                .get("agent_confirm")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            startup_snippet_id: uuid_member(ssh, "startup_snippet_id"),
            jump_chain: uuid_list(ssh.get("jump_chain")),
            proxy_kind: ProxyKind::parse(proxy.map(|proxy| str_of(proxy, "type"))),
            proxy_address: proxy
                .map(|proxy| str_of(proxy, "address").to_owned())
                .unwrap_or_default(),
            proxy_port: proxy
                .map(|proxy| int_text(proxy, "port"))
                .unwrap_or_default(),
            proxy_username: proxy
                .map(|proxy| str_of(proxy, "username").to_owned())
                .unwrap_or_default(),
            env,
            charset: str_of(data, "charset").to_owned(),
            keepalive: int_text(ssh, "keepalive_s"),
            record_history: OverrideMode::parse(str_of(data, "record_history")),
            record_output: OverrideMode::parse(str_of(data, "record_output")),
            mosh: ssh.get("mosh").and_then(Value::as_bool).unwrap_or(false),
            terminal_theme: data
                .get("terminal_theme")
                .and_then(|theme| theme.get("builtin"))
                .and_then(Value::as_str)
                .filter(|slug| TERMINAL_THEMES.contains(slug))
                .map(str::to_owned),
            telnet: data
                .get("telnet")
                .and_then(Value::as_object)
                .map(|telnet| TelnetDraft {
                    port: int_text(telnet, "port"),
                    username: str_of(telnet, "username").to_owned(),
                }),
        }
    }

    /// Errors the panel can tell without the core: address, ports,
    /// keepalive, tags, environment variable names and the proxy.
    pub fn validate(&self) -> FieldErrors {
        let mut errors = FieldErrors::new();
        match check_address(&self.address) {
            Ok(Some(_)) => {}
            Ok(None) => {
                errors.insert("address".into(), ValidationReason::Required);
            }
            Err(reason) => {
                errors.insert("address".into(), reason);
            }
        }
        if let Err(reason) = check_port(&self.port) {
            errors.insert("ssh.port".into(), reason);
        }
        if let Err(reason) = check_keepalive(&self.keepalive) {
            errors.insert("ssh.keepalive_s".into(), reason);
        }
        if self.tags.len() > MAX_TAGS {
            errors.insert("tags".into(), ValidationReason::TooMany);
        }
        let mut names = BTreeSet::new();
        for (index, var) in self.env.iter().enumerate() {
            let name = var.name.trim();
            if name.is_empty() && var.value.is_empty() {
                continue;
            }
            if !env_name_ok(name) {
                errors.insert(format!("ssh.env.{index}.name"), ValidationReason::Invalid);
            } else if !names.insert(name.to_owned()) {
                errors.insert(format!("ssh.env.{index}.name"), ValidationReason::Duplicate);
            }
        }
        if self.proxy_kind != ProxyKind::None {
            match check_address(&self.proxy_address) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    errors.insert("ssh.proxy.address".into(), ValidationReason::Required);
                }
                Err(reason) => {
                    errors.insert("ssh.proxy.address".into(), reason);
                }
            }
            if self.proxy_port.trim().is_empty() {
                errors.insert("ssh.proxy.port".into(), ValidationReason::Required);
            } else if let Err(reason) = check_port(&self.proxy_port) {
                errors.insert("ssh.proxy.port".into(), reason);
            }
        }
        if let Some(telnet) = &self.telnet {
            if let Err(reason) = check_port(&telnet.port) {
                errors.insert("telnet.port".into(), reason);
            }
            if telnet.username.trim().chars().count() > 256 {
                errors.insert("telnet.username".into(), ValidationReason::TooLong);
            }
        }
        if !self.charset.trim().is_empty() && !charset_ok(&self.charset) {
            errors.insert("charset".into(), ValidationReason::Invalid);
        }
        errors
    }

    /// Writes the form into `base` (the stored data object, or empty for a
    /// new host). Members the form does not show are kept as they are.
    pub fn apply(&self, base: &Map<String, Value>) -> Map<String, Value> {
        let mut data = base.clone();
        data.insert("address".into(), Value::from(self.address.trim()));
        set_text(&mut data, "label", &self.label);
        match self.group_id {
            Some(group) => {
                data.insert("group_id".into(), json!(group));
            }
            None => {
                data.remove("group_id");
            }
        }
        if self.tags.is_empty() {
            data.remove("tags");
        } else {
            data.insert("tags".into(), json!(self.tags));
        }
        if self.color == TagColor::Graphite && !base.contains_key("color") {
            data.remove("color");
        } else {
            data.insert("color".into(), json!(self.color));
        }
        if self.backspace == BackspaceMode::Default && !base.contains_key("backspace") {
            data.remove("backspace");
        } else {
            data.insert("backspace".into(), json!(self.backspace.as_str()));
        }
        // Notes keep their line breaks; only an all-blank text is removed.
        if self.notes.trim().is_empty() {
            data.remove("notes");
        } else {
            data.insert(
                "notes".into(),
                Value::from(self.notes.replace("\r\n", "\n")),
            );
        }
        set_text(&mut data, "charset", &self.charset.to_lowercase());
        match &self.terminal_theme {
            Some(slug) => {
                data.insert("terminal_theme".into(), json!({ "builtin": slug }));
            }
            None => {
                // A reference this form cannot show (an imported theme) is kept.
                if data
                    .get("terminal_theme")
                    .is_some_and(|theme| theme.get("builtin").is_some())
                {
                    data.remove("terminal_theme");
                }
            }
        }
        match &self.telnet {
            Some(telnet) => {
                let block = object_mut(&mut data, "telnet");
                set_int(block, "port", &telnet.port);
                set_text(block, "username", &telnet.username);
            }
            None => {
                // Removing Telnet removes the member (data-model 7.4); a
                // Telnet primary protocol cannot outlive it.
                data.remove("telnet");
                if data.get("primary_protocol").and_then(Value::as_str) == Some("telnet") {
                    data.remove("primary_protocol");
                }
            }
        }
        for (member, mode) in [
            ("record_history", self.record_history),
            ("record_output", self.record_output),
        ] {
            if mode == OverrideMode::Default && !base.contains_key(member) {
                data.remove(member);
            } else {
                data.insert(member.into(), json!(mode.as_str()));
            }
        }

        let ssh = object_mut(&mut data, "ssh");
        set_int(ssh, "port", &self.port);
        set_text(ssh, "username", &self.username);
        for (member, value) in [
            ("identity_id", self.identity_id),
            ("key_id", self.key_id),
            ("certificate_key_id", self.certificate_key_id),
            ("fido2_key_id", self.fido2_key_id),
        ] {
            match value {
                Some(id) => {
                    ssh.insert(member.into(), json!(id));
                }
                None => {
                    ssh.remove(member);
                }
            }
        }
        for (member, flag) in [
            ("agent_forwarding", self.agent_forwarding),
            ("agent_forward_system", self.agent_forward_system),
            ("agent_confirm", self.agent_confirm),
            ("mosh", self.mosh),
        ] {
            if flag {
                ssh.insert(member.into(), Value::Bool(true));
            } else {
                ssh.remove(member);
            }
        }
        match self.startup_snippet_id {
            Some(id) => {
                ssh.insert("startup_snippet_id".into(), json!(id));
            }
            None => {
                ssh.remove("startup_snippet_id");
            }
        }
        match &self.jump_chain {
            Some(chain) => {
                ssh.insert("jump_chain".into(), json!(chain));
            }
            None => {
                ssh.remove("jump_chain");
            }
        }
        set_int(ssh, "keepalive_s", &self.keepalive);
        let env: Vec<Value> = self
            .env
            .iter()
            .filter(|var| !(var.name.trim().is_empty() && var.value.is_empty()))
            .map(|var| json!({"name": var.name.trim(), "value": var.value}))
            .collect();
        if env.is_empty() {
            ssh.remove("env");
        } else {
            ssh.insert("env".into(), Value::Array(env));
        }
        match self.proxy_kind.as_str() {
            None => {
                ssh.remove("proxy");
            }
            Some(kind) => {
                let proxy = object_mut(ssh, "proxy");
                proxy.insert("type".into(), Value::from(kind));
                set_text(proxy, "address", &self.proxy_address);
                set_int(proxy, "port", &self.proxy_port);
                set_text(proxy, "username", &self.proxy_username);
                // The password is a secret member (`ssh.proxy.password`).
                proxy.remove("password");
            }
        }
        // Secret members never travel in `data`.
        ssh.remove("password");
        data
    }
}

/// The values of the group panel form (UI 3.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupDraft {
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub color: TagColor,
    pub port: String,
    pub username: String,
    /// `None`: inherit.
    pub jump_chain: Option<Vec<Uuid>>,
}

impl GroupDraft {
    pub fn from_group(group: &GroupItem) -> Self {
        let data = group.data_map();
        let empty = Map::new();
        let ssh = data.get("ssh").and_then(Value::as_object).unwrap_or(&empty);
        Self {
            name: group.name.clone(),
            parent_id: group.parent_id,
            color: color_of(&data),
            port: int_text(ssh, "port"),
            username: str_of(ssh, "username").to_owned(),
            jump_chain: uuid_list(ssh.get("jump_chain")),
        }
    }

    pub fn validate(&self) -> FieldErrors {
        let mut errors = FieldErrors::new();
        match text::string(&self.name, 256) {
            Ok(Some(_)) => {}
            Ok(None) => {
                errors.insert("name".into(), ValidationReason::Required);
            }
            Err(reason) => {
                errors.insert("name".into(), reason);
            }
        }
        if let Err(reason) = check_port(&self.port) {
            errors.insert("ssh.port".into(), reason);
        }
        errors
    }

    pub fn apply(&self, base: &Map<String, Value>) -> Map<String, Value> {
        let mut data = base.clone();
        data.insert("name".into(), Value::from(self.name.trim()));
        match self.parent_id {
            Some(parent) => {
                data.insert("parent_id".into(), json!(parent));
            }
            None => {
                data.remove("parent_id");
            }
        }
        if self.color == TagColor::Graphite && !base.contains_key("color") {
            data.remove("color");
        } else {
            data.insert("color".into(), json!(self.color));
        }
        let ssh = object_mut(&mut data, "ssh");
        set_int(ssh, "port", &self.port);
        set_text(ssh, "username", &self.username);
        match &self.jump_chain {
            Some(chain) => {
                ssh.insert("jump_chain".into(), json!(chain));
            }
            None => {
                ssh.remove("jump_chain");
            }
        }
        data
    }
}

/// The result of saving a panel form.
#[derive(Clone, Debug, Default)]
pub struct SaveReport<T> {
    /// The stored record after the save; `None` while the form is a draft or
    /// when nothing could be written.
    pub saved: Option<T>,
    /// Members that were refused (shown under their fields, not written).
    pub errors: FieldErrors,
}

fn errors_from(fields: Vec<FieldError>) -> FieldErrors {
    fields
        .into_iter()
        .map(|error| (error.field, error.reason))
        .collect()
}

// -- AppState additions --------------------------------------------------------------

impl AppState {
    /// What a record inside `parent` inherits (`None`: the top level, nothing).
    pub fn inherited_ssh(&self, parent: Option<Uuid>) -> InheritedSsh {
        let mut inherited = InheritedSsh::default();
        let mut level = parent;
        let mut seen = 0;
        // Only read when a level names an identity.
        let mut identities: Option<Vec<IdentityView>> = None;
        while let Some(group_id) = level {
            if seen >= MAX_CHAIN {
                break;
            }
            seen += 1;
            let Some(group) = self.group(group_id) else {
                break;
            };
            if let Some(ssh) = group.data.get("ssh").and_then(Value::as_object) {
                let idf = match uuid_member(ssh, "identity_id") {
                    Some(id) => identities
                        .get_or_insert_with(|| self.identities())
                        .iter()
                        .find(|identity| identity.record_id == id)
                        .cloned(),
                    None => None,
                };
                inherited.absorb(ssh, group_id, idf.as_ref());
            }
            level = group.parent_id;
        }
        inherited
    }

    /// `group` and every group below it.
    pub fn group_subtree(&self, group: Uuid) -> Vec<Uuid> {
        let mut out = vec![group];
        let mut index = 0;
        while index < out.len() {
            let current = out[index];
            out.extend(
                self.groups
                    .iter()
                    .filter(|candidate| candidate.parent_id == Some(current))
                    .map(|candidate| candidate.record_id)
                    .filter(|id| !out.contains(id))
                    .collect::<Vec<_>>(),
            );
            index += 1;
        }
        out
    }

    /// Groups a group can move into: every group except itself and its
    /// subtree (data-model 7.1). The top level is always a valid target too.
    pub fn group_move_targets(&self, group: Uuid) -> Vec<&GroupItem> {
        let subtree = self.group_subtree(group);
        self.groups
            .iter()
            .filter(|candidate| !subtree.contains(&candidate.record_id))
            .collect()
    }

    /// "Production / Databases": the names from the top level down to `group`.
    pub fn group_path_label(&self, group: Uuid) -> String {
        let Some(item) = self.group(group) else {
            return String::new();
        };
        let mut names: Vec<&str> = item
            .path
            .iter()
            .filter_map(|id| self.group(*id))
            .map(|group| group.name.as_str())
            .collect();
        names.push(item.name.as_str());
        names.join(" / ")
    }

    /// Every tag used in the vault, de-duplicated case-insensitively (the
    /// first spelling wins), sorted for autocomplete (UI 3.1 Tags).
    pub fn all_tags(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut tags = Vec::new();
        for host in self.hosts.iter() {
            for tag in &host.tags {
                if seen.insert(text::fold(tag)) {
                    tags.push(tag.clone());
                }
            }
        }
        tags.sort_by_key(|tag| text::fold(tag));
        tags
    }

    /// Creates a group with just a name (Parent Group "create on the fly",
    /// UI 3.1) inside `parent`.
    pub fn create_group_named(
        &mut self,
        name: &str,
        parent: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> Result<GroupItem, CoreError> {
        let mut data = Map::new();
        data.insert("name".into(), Value::from(name.trim()));
        if let Some(parent) = parent {
            data.insert("parent_id".into(), json!(parent));
        }
        self.save_group(None, data, cx)
    }

    /// Creates or updates a host with secret members (`ssh.password`,
    /// `ssh.proxy.password`): `Some` sets a value, `None` clears it, a missing
    /// path keeps the stored value.
    pub fn save_host_with_secrets(
        &mut self,
        record_id: Option<Uuid>,
        data: Map<String, Value>,
        secrets: BTreeMap<String, Option<SecretString>>,
        cx: &mut Context<Self>,
    ) -> Result<HostItem, CoreError> {
        let vault_id = self.require_vault()?;
        let view = self.require_core()?.record_save(RecordSaveInput {
            vault_id,
            record_id,
            record_type: RecordType::Host,
            data,
            secrets,
        })?;
        self.records_written(cx);
        match view {
            RecordView::Host(view) => Ok(HostItem::from_view(&view)),
            RecordView::Group(_) => Err(CoreError::NotFound),
        }
    }

    /// Reads a stored secret member (the password's "show" button).
    pub fn reveal_secret(&self, record_id: Uuid, path: &str) -> Result<SecretString, CoreError> {
        let vault_id = self.require_vault()?;
        self.require_core()?
            .secret_reveal(shellpp_core::service::dto::SecretFieldRef {
                vault_id,
                record_id,
                path: path.to_owned(),
            })
    }

    /// Saves the host panel form by the saving rule (module docs). `record_id`
    /// `None` creates the host once the address is valid; the new host is
    /// selected so its card shows as selected (UI 3.2).
    pub fn save_host_draft(
        &mut self,
        record_id: Option<Uuid>,
        draft: &HostDraft,
        secrets: &HostSecrets,
        cx: &mut Context<Self>,
    ) -> SaveReport<HostItem> {
        let mut errors = draft.validate();
        if record_id.is_none() && errors.contains_key("address") {
            // Still a draft (UI 1.3): nothing is written. An empty address
            // is not an error yet.
            if draft.address.trim().is_empty() {
                errors.remove("address");
            }
            return SaveReport {
                saved: None,
                errors,
            };
        }
        let base = record_id
            .and_then(|id| self.host(id))
            .map(HostItem::data_map)
            .unwrap_or_default();
        let mut data = draft.apply(&base);
        for path in errors.keys() {
            restore_member(&mut data, &base, &member_of(path));
        }
        let mut secret_map = BTreeMap::new();
        for (path, change) in [
            ("ssh.password", &secrets.password),
            ("ssh.proxy.password", &secrets.proxy_password),
        ] {
            match change {
                SecretChange::Keep => {}
                SecretChange::Set(value) => {
                    secret_map.insert(path.to_owned(), Some(value.clone()));
                }
                SecretChange::Clear => {
                    if record_id.is_some() {
                        secret_map.insert(path.to_owned(), None);
                    }
                }
            }
        }
        if draft.proxy_kind == ProxyKind::None {
            secret_map.remove("ssh.proxy.password");
        }
        let mut secrets = secret_map;
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.save_host_with_secrets(record_id, data.clone(), secrets.clone(), cx) {
                Ok(host) => {
                    if record_id.is_none() {
                        self.select(host.record_id, SelectMode::Replace, &[], cx);
                    }
                    return SaveReport {
                        saved: Some(host),
                        errors,
                    };
                }
                Err(CoreError::Validation(fields)) => {
                    let refused = errors_from(fields);
                    tracing::info!(fields = refused.len(), "host member refused");
                    let address_refused = refused.contains_key("address");
                    for path in refused.keys() {
                        secrets.remove(path.as_str());
                        restore_member(&mut data, &base, &member_of(path));
                    }
                    errors.extend(refused);
                    if attempt >= 2 || (record_id.is_none() && address_refused) {
                        return SaveReport {
                            saved: None,
                            errors,
                        };
                    }
                }
                Err(error) => {
                    tracing::warn!(code = error.code(), "host not saved");
                    return SaveReport {
                        saved: None,
                        errors,
                    };
                }
            }
        }
    }

    // -- Keychain records: the host panel's "+" and the Keychain (UI 3.1, 7) -----

    /// Every identity of the personal vault, by label (empty while locked).
    pub fn identities(&self) -> Vec<IdentityView> {
        self.require_core()
            .and_then(|core| core.identities_list())
            .unwrap_or_else(|error| {
                tracing::warn!(code = error.code(), "identities not listed");
                Vec::new()
            })
    }

    /// Every key of the personal vault, by label (empty while locked).
    pub fn keys(&self) -> Vec<KeyView> {
        self.require_core()
            .and_then(|core| core.keys_list())
            .unwrap_or_else(|error| {
                tracing::warn!(code = error.code(), "keys not listed");
                Vec::new()
            })
    }

    /// Creates (`record_id` `None`) or updates an identity: label (required),
    /// username, password (UI 3.1 "+ -> Identity", 7.4).
    pub fn save_identity(
        &mut self,
        record_id: Option<Uuid>,
        label: &str,
        username: &str,
        password: PasswordChange,
        cx: &mut Context<Self>,
    ) -> Result<IdentityView, CoreError> {
        let vault_id = self.require_vault()?;
        let key_id = record_id
            .and_then(|id| {
                self.identities()
                    .into_iter()
                    .find(|view| view.record_id == id)
            })
            .and_then(|view| view.key_id);
        let view = self.require_core()?.identity_save(IdentitySaveInput {
            vault_id,
            record_id,
            label: label.to_owned(),
            username: username.to_owned(),
            key_id,
            password,
        })?;
        cx.notify();
        Ok(view)
    }

    /// Imports an OpenSSH private key (pasted or read from a file) as a key
    /// record (UI 3.1 "+ -> Key", 7.3). An encrypted key needs its
    /// passphrase (`passphrase` `required` otherwise); `save_passphrase`
    /// stores it, otherwise it is asked on each connection. Decryption
    /// (bcrypt-pbkdf) runs on the background executor.
    pub fn import_key(
        &mut self,
        private_key: SecretString,
        label: Option<String>,
        passphrase: Option<SecretString>,
        save_passphrase: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<KeyImportResult, CoreError>> {
        let (core, vault_id) = match (self.require_core(), self.require_vault()) {
            (Ok(core), Ok(vault_id)) => (core.clone(), vault_id),
            (Err(error), _) | (_, Err(error)) => return Task::ready(Err(error)),
        };
        let work = cx.background_spawn(async move {
            core.key_import(KeyImportInput {
                vault_id,
                label,
                private_key,
                passphrase,
                save_passphrase,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            match &result {
                Ok(imported) => tracing::info!(created = imported.created, "key imported"),
                Err(error) => tracing::info!(code = error.code(), "key not imported"),
            }
            let _ = this.update(cx, |_, cx| cx.notify());
            result
        })
    }

    /// Generate key (UI 7.1, D32): a new key pair of `kind` labelled
    /// `label`, encrypted with `passphrase` when one is given and stored like
    /// an import (`save_passphrase` as there). RSA takes seconds: the work
    /// runs on the background executor.
    pub fn generate_key(
        &mut self,
        label: String,
        kind: KeyKind,
        passphrase: Option<SecretString>,
        save_passphrase: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<KeyImportResult, CoreError>> {
        let (core, vault_id) = match (self.require_core(), self.require_vault()) {
            (Ok(core), Ok(vault_id)) => (core.clone(), vault_id),
            (Err(error), _) | (_, Err(error)) => return Task::ready(Err(error)),
        };
        let work = cx.background_spawn(async move {
            core.key_generate(KeyGenerateInput {
                vault_id,
                label,
                kind,
                passphrase,
                save_passphrase,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            match &result {
                Ok(_) => tracing::info!(kind = kind.algorithm_name(), "key generated"),
                Err(error) => tracing::info!(code = error.code(), "key not generated"),
            }
            let _ = this.update(cx, |_, cx| cx.notify());
            result
        })
    }

    fn key_ref(&self, key_id: Uuid) -> Result<RecordRef, CoreError> {
        Ok(RecordRef {
            vault_id: self.require_vault()?,
            record_id: key_id,
        })
    }

    /// Renames a key (the key panel's Label, saved as in UI 1.3).
    pub fn rename_key(
        &mut self,
        key_id: Uuid,
        label: &str,
        cx: &mut Context<Self>,
    ) -> Result<KeyView, CoreError> {
        let key = self.key_ref(key_id)?;
        let view = self.require_core()?.key_label_set(KeyLabelInput {
            key,
            label: label.to_owned(),
        })?;
        cx.notify();
        Ok(view)
    }

    /// What the key panel shows of a key's certificate (UI 7.3): `None`
    /// without one.
    pub fn key_certificate_info(
        &self,
        key_id: Uuid,
    ) -> Result<Option<shellpp_core::keys::certificate::CertificateInfo>, CoreError> {
        let key = self.key_ref(key_id)?;
        self.require_core()?.key_certificate_info(key)
    }

    /// Removes a key's certificate.
    pub fn remove_key_certificate(
        &mut self,
        key_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Result<KeyView, CoreError> {
        let key = self.key_ref(key_id)?;
        let view = self.require_core()?.key_certificate_set(
            shellpp_core::service::keychain::KeyCertificateInput {
                key,
                certificate: String::new(),
            },
        )?;
        cx.notify();
        Ok(view)
    }

    /// The Certificate action (UI 7.1): the certificate goes onto the key
    /// of the personal vault it certifies.
    pub fn import_certificate(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<KeyView, CoreError> {
        let vault = self.require_vault()?;
        let view = self.require_core()?.certificate_import(vault, text)?;
        cx.notify();
        Ok(view)
    }

    /// The one-line public key of a key (not a secret, UI 7.3).
    pub fn key_public_key(&self, key_id: Uuid) -> Result<String, CoreError> {
        let key = self.key_ref(key_id)?;
        self.require_core()?.key_public_key(key)
    }

    /// The hosts, groups and identities that use a key: the confirmation of
    /// its Remove lists them (D50).
    pub fn key_users(
        &self,
        key_id: Uuid,
    ) -> Result<shellpp_core::service::keychain::KeyUsers, CoreError> {
        let key = self.key_ref(key_id)?;
        self.require_core()?.key_users(key)
    }

    /// The private key text, for the key panel's explicit Show (UI 7.3).
    /// The caller keeps it only while it is shown.
    pub fn reveal_private_key(&self, key_id: Uuid) -> Result<SecretString, CoreError> {
        let key = self.key_ref(key_id)?;
        self.require_core()?.secret_reveal(SecretFieldRef {
            vault_id: key.vault_id,
            record_id: key.record_id,
            path: "private_key".to_owned(),
        })
    }

    /// Copy private key (UI 7.3): the core writes it to the OS clipboard as
    /// a sensitive value and clears it after 30 seconds if the clipboard
    /// still holds it (docs/security.md, section 9.4).
    pub fn copy_private_key(&self, key_id: Uuid) -> Result<(), CoreError> {
        let key = self.key_ref(key_id)?;
        self.require_core()?
            .secret_copy(SecretCopyTarget::RecordField {
                vault_id: key.vault_id,
                record_id: key.record_id,
                path: "private_key".to_owned(),
            })
    }

    /// Stores the OpenSSH user certificate of a key (UI 3.1 "+ ->
    /// Certificate"; data-model 9.2); it must certify that key.
    pub fn set_key_certificate(
        &mut self,
        key_id: Uuid,
        certificate: &str,
        cx: &mut Context<Self>,
    ) -> Result<KeyView, CoreError> {
        let vault_id = self.require_vault()?;
        let view = self
            .require_core()?
            .key_certificate_set(KeyCertificateInput {
                key: RecordRef {
                    vault_id,
                    record_id: key_id,
                },
                certificate: certificate.to_owned(),
            })?;
        cx.notify();
        Ok(view)
    }

    /// Whether a key holds a certificate.
    pub fn key_has_certificate(&self, key_id: Uuid) -> bool {
        let Ok(vault_id) = self.require_vault() else {
            return false;
        };
        self.require_core()
            .and_then(|core| {
                core.key_has_certificate(RecordRef {
                    vault_id,
                    record_id: key_id,
                })
            })
            .unwrap_or(false)
    }

    /// Saves the group panel form by the saving rule. `record_id` `None`
    /// creates the group once the name is valid.
    pub fn save_group_draft(
        &mut self,
        record_id: Option<Uuid>,
        draft: &GroupDraft,
        cx: &mut Context<Self>,
    ) -> SaveReport<GroupItem> {
        let mut errors = draft.validate();
        if record_id.is_none() && errors.contains_key("name") {
            if draft.name.trim().is_empty() {
                errors.remove("name");
            }
            return SaveReport {
                saved: None,
                errors,
            };
        }
        let base = record_id
            .and_then(|id| self.group(id))
            .map(GroupItem::data_map)
            .unwrap_or_default();
        let mut data = draft.apply(&base);
        for path in errors.keys() {
            restore_member(&mut data, &base, &member_of(path));
        }
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.save_group(record_id, data.clone(), cx) {
                Ok(group) => {
                    return SaveReport {
                        saved: Some(group),
                        errors,
                    };
                }
                Err(CoreError::Validation(fields)) => {
                    let refused = errors_from(fields);
                    let name_refused = refused.contains_key("name");
                    for path in refused.keys() {
                        restore_member(&mut data, &base, &member_of(path));
                    }
                    errors.extend(refused);
                    if attempt >= 2 || (record_id.is_none() && name_refused) {
                        return SaveReport {
                            saved: None,
                            errors,
                        };
                    }
                }
                Err(error) => {
                    tracing::warn!(code = error.code(), "group not saved");
                    return SaveReport {
                        saved: None,
                        errors,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::testing::TestProfile;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn member_paths_cover_list_items_and_the_proxy() {
        assert_eq!(member_of("address"), "address");
        assert_eq!(member_of("tags.3"), "tags");
        assert_eq!(member_of("ssh.env.2.name"), "ssh.env");
        assert_eq!(member_of("ssh.proxy.port"), "ssh.proxy");
        assert_eq!(member_of("ssh.port"), "ssh.port");
    }

    #[test]
    fn port_and_keepalive_checks() {
        assert!(check_port("").is_ok());
        assert!(check_port("22").is_ok());
        assert_eq!(check_port("0"), Err(ValidationReason::OutOfRange));
        assert_eq!(check_port("70000"), Err(ValidationReason::OutOfRange));
        assert_eq!(check_port("ssh"), Err(ValidationReason::Invalid));
        assert!(check_keepalive("0").is_ok());
        assert_eq!(check_keepalive("3601"), Err(ValidationReason::OutOfRange));
    }

    #[test]
    fn draft_round_trips_through_data() {
        let data = map(json!({
            "address": "db1.example.com",
            "label": "DB",
            "tags": ["prod", "db"],
            "color": "ocean",
            "backspace": "ctrl_h",
            "notes": "line 1\nline 2",
            "charset": "utf-8",
            "record_history": "off",
            "terminal_theme": {"builtin": "paper"},
            "telnet": {"port": 2323, "username": "admin"},
            "custom": {"kept": true},
            "ssh": {
                "port": 2222,
                "username": "admin",
                "agent_forwarding": true,
                "mosh": true,
                "startup_snippet_id": "0199f6a1-0000-7000-8000-000000000004",
                "keepalive_s": 30,
                "env": [{"name": "LANG", "value": "C"}],
                "proxy": {"type": "socks5", "address": "proxy.example.com", "port": 1080},
                "identity_id": "0199f6a1-0000-7000-8000-000000000002"
            }
        }));
        let draft = HostDraft::from_data(&data, None);
        assert_eq!(draft.port, "2222");
        assert_eq!(draft.backspace, BackspaceMode::CtrlH);
        assert_eq!(draft.record_history, OverrideMode::Off);
        assert_eq!(draft.proxy_kind, ProxyKind::Socks5);
        assert!(draft.mosh);
        assert_eq!(draft.terminal_theme.as_deref(), Some("paper"));
        assert_eq!(
            draft.telnet,
            Some(TelnetDraft {
                port: "2323".into(),
                username: "admin".into()
            })
        );
        assert!(draft.startup_snippet_id.is_some());
        assert!(draft.validate().is_empty());
        let applied = draft.apply(&data);
        assert_eq!(applied, data);
    }

    #[test]
    fn telnet_charset_and_theme_members() {
        let base = map(json!({
            "address": "h.example.com",
            "primary_protocol": "telnet",
            "telnet": {"port": 23, "x_kept": 1}
        }));
        let mut draft = HostDraft::from_data(&base, None);
        // Edits keep unknown members of the block.
        draft.telnet = Some(TelnetDraft {
            port: String::new(),
            username: " root ".into(),
        });
        draft.charset = "KOI8-R".into();
        let data = draft.apply(&base);
        assert_eq!(data["telnet"], json!({"username": "root", "x_kept": 1}));
        assert_eq!(data["charset"], json!("koi8-r"));
        // Removing Telnet removes the member and a Telnet primary protocol.
        draft.telnet = None;
        let data = draft.apply(&base);
        assert!(data.get("telnet").is_none());
        assert!(data.get("primary_protocol").is_none());
        // Client checks of the new members.
        draft.telnet = Some(TelnetDraft {
            port: "70000".into(),
            username: String::new(),
        });
        draft.charset = "utf 8".into();
        let errors = draft.validate();
        assert_eq!(
            errors.get("telnet.port"),
            Some(&ValidationReason::OutOfRange)
        );
        assert_eq!(errors.get("charset"), Some(&ValidationReason::Invalid));
        assert_eq!(member_of("telnet.port"), "telnet.port");
        // A theme reference the form cannot show is kept.
        let imported = map(json!({
            "address": "h.example.com",
            "terminal_theme": {"record_id": "0199f6a1-0000-7000-8000-000000000005"}
        }));
        let kept = HostDraft::from_data(&imported, None).apply(&imported);
        assert_eq!(kept["terminal_theme"], imported["terminal_theme"]);
        assert!(CHARSETS.iter().all(|label| charset_ok(label)));
        assert_eq!(TERMINAL_THEMES[..2], ["graphite", "paper"]);
    }

    #[test]
    fn draft_reports_invalid_members() {
        let draft = HostDraft {
            address: "bad address".into(),
            port: "99999".into(),
            env: vec![
                EnvVar {
                    name: "1BAD".into(),
                    value: "x".into(),
                },
                EnvVar {
                    name: "OK".into(),
                    value: "1".into(),
                },
                EnvVar {
                    name: "OK".into(),
                    value: "2".into(),
                },
            ],
            ..HostDraft::default()
        };
        let errors = draft.validate();
        assert_eq!(errors.get("address"), Some(&ValidationReason::Invalid));
        assert_eq!(errors.get("ssh.port"), Some(&ValidationReason::OutOfRange));
        assert_eq!(
            errors.get("ssh.env.0.name"),
            Some(&ValidationReason::Invalid)
        );
        assert_eq!(
            errors.get("ssh.env.2.name"),
            Some(&ValidationReason::Duplicate)
        );
    }

    #[gpui_kit::test]
    fn draft_is_created_on_the_first_valid_address_and_invalid_members_stay_out(
        cx: &mut TestAppContext,
    ) {
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            // Empty address: still a draft, no error shown.
            let report =
                state.save_host_draft(None, &HostDraft::default(), &HostSecrets::default(), cx);
            assert!(report.saved.is_none());
            assert!(report.errors.is_empty());
            assert!(state.hosts().is_empty());

            // Invalid address: still a draft, the reason is reported.
            let mut draft = HostDraft {
                address: "a b".into(),
                ..HostDraft::default()
            };
            let report = state.save_host_draft(None, &draft, &HostSecrets::default(), cx);
            assert!(report.saved.is_none());
            assert_eq!(
                report.errors.get("address"),
                Some(&ValidationReason::Invalid)
            );
            assert!(state.hosts().is_empty());

            // Valid address: created and selected.
            draft.address = "192.0.2.10".into();
            draft.label = "Web".into();
            let report = state.save_host_draft(None, &draft, &HostSecrets::default(), cx);
            let host = report.saved.expect("created");
            assert_eq!(state.hosts().len(), 1);
            assert!(state.selection().contains(&host.record_id));

            // An invalid port is not written, the valid label change is.
            draft.label = "Web 2".into();
            draft.port = "70000".into();
            let report =
                state.save_host_draft(Some(host.record_id), &draft, &HostSecrets::default(), cx);
            assert_eq!(
                report.errors.get("ssh.port"),
                Some(&ValidationReason::OutOfRange)
            );
            let stored = state.host(host.record_id).unwrap();
            assert_eq!(stored.label.as_deref(), Some("Web 2"));
            assert_eq!(stored.port, None);

            // The address can never become invalid: the last valid one stays.
            draft.port = "2222".into();
            draft.address = "bad/address".into();
            let report =
                state.save_host_draft(Some(host.record_id), &draft, &HostSecrets::default(), cx);
            assert!(report.errors.contains_key("address"));
            let stored = state.host(host.record_id).unwrap();
            assert_eq!(stored.address, "192.0.2.10");
            assert_eq!(stored.port, Some(2222));

            // The password is a secret member.
            draft.address = "192.0.2.10".into();
            let report = state.save_host_draft(
                Some(host.record_id),
                &draft,
                &HostSecrets {
                    password: SecretChange::Set(SecretString::from("hunter2-test")),
                    ..HostSecrets::default()
                },
                cx,
            );
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            let stored = state.host(host.record_id).unwrap();
            assert!(stored.secrets_set.iter().any(|path| path == "ssh.password"));
            let revealed = state.reveal_secret(host.record_id, "ssh.password").unwrap();
            assert_eq!(revealed.expose_secret(), "hunter2-test");
            state.save_host_draft(
                Some(host.record_id),
                &draft,
                &HostSecrets {
                    password: SecretChange::Clear,
                    ..HostSecrets::default()
                },
                cx,
            );
            let stored = state.host(host.record_id).unwrap();
            assert!(stored.secrets_set.is_empty());
            state.shutdown();
        });
    }

    #[gpui_kit::test]
    fn inheritance_comes_from_the_nearest_group(cx: &mut TestAppContext) {
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            let top = state
                .save_group(
                    None,
                    map(json!({"name": "Production", "ssh": {"port": 2222, "username": "ops"}})),
                    cx,
                )
                .unwrap();
            let inner = state
                .save_group(
                    None,
                    map(json!({"name": "Databases", "parent_id": top.record_id, "ssh": {"username": "dba"}})),
                    cx,
                )
                .unwrap();
            let inherited = state.inherited_ssh(Some(inner.record_id));
            assert_eq!(
                inherited.username,
                Some(InheritedFrom {
                    value: "dba".into(),
                    group_id: inner.record_id
                })
            );
            assert_eq!(
                inherited.port,
                Some(InheritedFrom {
                    value: 2222,
                    group_id: top.record_id
                })
            );
            assert_eq!(state.inherited_ssh(None), InheritedSsh::default());
            assert_eq!(
                state.group_path_label(inner.record_id),
                "Production / Databases"
            );
            // A group never moves into its own subtree.
            let targets: Vec<Uuid> = state
                .group_move_targets(top.record_id)
                .iter()
                .map(|group| group.record_id)
                .collect();
            assert!(targets.is_empty());
            let targets: Vec<Uuid> = state
                .group_move_targets(inner.record_id)
                .iter()
                .map(|group| group.record_id)
                .collect();
            assert_eq!(targets, vec![top.record_id]);
            state.shutdown();
        });
    }

    #[gpui_kit::test]
    fn group_draft_and_tags(cx: &mut TestAppContext) {
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            let report = state.save_group_draft(None, &GroupDraft::default(), cx);
            assert!(report.saved.is_none() && report.errors.is_empty());
            let mut draft = GroupDraft {
                name: "Lab".into(),
                port: "0".into(),
                ..GroupDraft::default()
            };
            let report = state.save_group_draft(None, &draft, cx);
            let group = report.saved.expect("created");
            assert_eq!(
                report.errors.get("ssh.port"),
                Some(&ValidationReason::OutOfRange)
            );
            // An empty name is refused; the stored name stays.
            draft.name = " ".into();
            draft.port = "22".into();
            let report = state.save_group_draft(Some(group.record_id), &draft, cx);
            assert_eq!(report.errors.get("name"), Some(&ValidationReason::Required));
            let stored = state.group(group.record_id).unwrap();
            assert_eq!(stored.name, "Lab");

            for (address, tags) in [
                ("a.example.com", json!(["Prod", "db"])),
                ("b.example.com", json!(["prod", "web"])),
            ] {
                state
                    .save_host(None, map(json!({"address": address, "tags": tags})), cx)
                    .unwrap();
            }
            assert_eq!(state.all_tags(), vec!["db", "Prod", "web"]);
            assert!(has_tag(&["Prod".to_owned()], "prod"));
            state.shutdown();
        });
    }
}
