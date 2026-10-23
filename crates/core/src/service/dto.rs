//! IPC data transfer objects: the JSON shapes the webview sends and receives
//! (docs/architecture.md, sections 3.4 and 3.5).
//!
//! This file and `apps/desktop/src/ipc/types.ts` describe the same contract.
//! Change both together and record the change in the stage plan.
//!
//! Conventions:
//! - JSON field names are snake_case; enums are snake_case strings; tagged
//!   unions use `"kind"` (or `"type"` for record views).
//! - Output `Option` fields are omitted when `None`; input `Option` fields
//!   accept a missing member or `null`.
//! - UUIDs are lowercase hyphenated text, timestamps RFC 3339 UTC with
//!   milliseconds (`2026-10-18T22:31:05.123Z`).
//! - Secrets (`SecretString`) are accepted only where listed and returned only
//!   by explicit reveals (`RecoveryKeyResult`, `SecretValue`).
//! - `Debug` never prints secrets, host data or search text.
//!
//! Owner: W2 in stage 1; the stage 2 section at the end is the lead's S4/S5
//! contract (stage 2 plan, section 5). Change shapes only through the plan.

use std::collections::BTreeMap;
use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::platform::OsKind;

fn expose<S: Serializer>(secret: &SecretString, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

/// Tile color of hosts, groups and vaults (docs/design-system.md, section 3.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagColor {
    #[default]
    Graphite,
    Jade,
    Amber,
    Coral,
    Plum,
    Moss,
    Ocean,
    Rose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    Personal,
    Team,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultStatus {
    Active,
    Archived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

/// Record types the webview can query and edit in stage 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordType {
    Host,
    Group,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// `app_info`. Filled by the desktop host.
#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    /// Always "Shell++".
    pub name: String,
    /// Semver of the app (Cargo package version).
    pub version: String,
    pub os: OsKind,
    /// `std::env::consts::ARCH`.
    pub arch: String,
    /// Debug build (`cfg(debug_assertions)`).
    pub debug: bool,
    /// Built with the `e2e-mocks` feature.
    pub e2e: bool,
}

/// One vault as the UI sees it. `name` and `color` come from `vault_meta`.
#[derive(Clone, Debug, Serialize)]
pub struct VaultView {
    pub vault_id: Uuid,
    pub kind: VaultKind,
    pub status: VaultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    pub name: String,
    pub color: TagColor,
}

/// The core part of `state_snapshot` (docs/architecture.md, section 3.5). The
/// desktop host adds `window_label` with `#[serde(flatten)]`.
#[derive(Clone, Debug, Serialize)]
pub struct CoreSnapshot {
    /// State version: events with `v` greater than this are newer than the snapshot.
    pub v: u64,
    pub lock: LockStatus,
    pub settings: SettingsValues,
    pub notifications: NotificationList,
    /// Empty while locked or at first run.
    pub vaults: Vec<VaultView>,
}

// ---------------------------------------------------------------------------
// Lock
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    /// No `local_auth` row yet: show the first-run screen (UI 13.1).
    FirstRun,
    /// Keys wiped: show the unlock screen (UI 13.2).
    Locked,
    Unlocked,
}

/// `lock_status`, and the result of `unlock`, `lock`, `recovery_unlock` and
/// `remember_device_set`. Readable in every state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LockStatus {
    pub state: LockState,
    /// An OS keychain is usable; otherwise "Remember on this device" is hidden (UI 13.4).
    pub remember_device_available: bool,
    /// A `device` key wrap exists for the current personal epoch.
    pub remember_device_enabled: bool,
}

/// Input of `first_run_setup`. The UI checks that the confirmation matches.
#[derive(Deserialize)]
pub struct FirstRunInput {
    pub password: SecretString,
}

/// Input of `unlock`, `recovery_key_show` and `recovery_key_reissue`.
#[derive(Deserialize)]
pub struct PasswordInput {
    pub password: SecretString,
}

/// Input of `recovery_unlock` (UI 13.2, "Use recovery key").
#[derive(Deserialize)]
pub struct RecoveryUnlockInput {
    /// Text as typed: `RK1-` prefix, spaces, hyphens and case are ignored.
    pub recovery_key: SecretString,
    pub new_password: SecretString,
}

/// Input of `password_change` (UI 13.4).
#[derive(Deserialize)]
pub struct PasswordChangeInput {
    pub current_password: SecretString,
    pub new_password: SecretString,
}

/// Input of `remember_device_set` (UI 13.4).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RememberDeviceInput {
    pub enabled: bool,
}

/// Result of `first_run_setup`, `recovery_key_show` and `recovery_key_reissue`:
/// the recovery key as `RK1-XXXXX-...` (docs/security.md, section 5.8).
#[derive(Serialize)]
pub struct RecoveryKeyResult {
    #[serde(serialize_with = "expose")]
    pub recovery_key: SecretString,
}

macro_rules! redacted_debug {
    ($($name:ident),* $(,)?) => {
        $(impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), " { .. }"))
            }
        })*
    };
}

redacted_debug!(
    FirstRunInput,
    PasswordInput,
    RecoveryUnlockInput,
    PasswordChangeInput,
    RecoveryKeyResult
);

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Which container a query covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryScope {
    /// Every record of the requested types in the vault.
    #[default]
    All,
    /// Records directly at the top level (hosts without a resolved group,
    /// groups treated as top-level by the tree rules).
    TopLevel,
    /// Records directly inside the group.
    Group { group_id: Uuid },
}

/// Sort modes of the Hosts screen (UI 2.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    /// Manual order keys (docs/data-model.md, section 18.1).
    #[default]
    Manual,
    Name,
    Created,
    Modified,
}

/// Input of `records_query`. Rules in the plan, section "records_query".
#[derive(Clone, Deserialize)]
pub struct RecordsQuery {
    /// Default: the personal vault.
    #[serde(default)]
    pub vault_id: Option<Uuid>,
    pub types: Vec<RecordType>,
    #[serde(default)]
    pub scope: QueryScope,
    /// Case-insensitive substring search.
    #[serde(default)]
    pub text: Option<String>,
    /// Hosts must carry every listed tag (case-insensitive).
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub sort: SortMode,
}

impl fmt::Debug for RecordsQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordsQuery")
            .field("vault_id", &self.vault_id)
            .field("types", &self.types)
            .field("scope", &self.scope)
            .field("text", &self.text.as_ref().map(|_| ".."))
            .field("tags", &self.tags.len())
            .field("sort", &self.sort)
            .finish()
    }
}

/// A value inherited from an ancestor group (UI 3.1, 3.4: "from group X").
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Inherited<T> {
    pub value: T,
    /// The group whose `ssh` block supplied the value.
    pub from_group_id: Uuid,
}

/// Values a host or group would inherit from its ancestors, ignoring its own
/// level (docs/data-model.md, section 8). A field is omitted when no ancestor sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct InheritedValues {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<Inherited<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<Inherited<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<Inherited<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<Inherited<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump_chain: Option<Inherited<Vec<Uuid>>>,
}

/// A host as the webview sees it.
#[derive(Clone, Serialize)]
pub struct HostView {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub revision: u64,
    /// Timestamp of the UUIDv7 record id.
    pub created_at: String,
    /// Milliseconds part of the payload HLC.
    pub modified_at: String,
    /// The `host` data object (docs/data-model.md, section 7.2) without secret
    /// members (`ssh.password`, `ssh.proxy.password`). Unknown members are kept.
    pub data: Value,
    /// Dotted paths of secret members that hold a value, for example `["ssh.password"]`.
    pub secrets_set: Vec<String>,
    pub inherited: InheritedValues,
    /// Ancestor group ids, top level first, ending with the host's own group.
    pub path: Vec<Uuid>,
}

/// A group as the webview sees it.
#[derive(Clone, Serialize)]
pub struct GroupView {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub revision: u64,
    pub created_at: String,
    pub modified_at: String,
    /// The `group` data object (docs/data-model.md, section 7.1).
    pub data: Value,
    pub inherited: InheritedValues,
    /// Ancestor group ids, top level first, ending with the parent (empty at top level).
    pub path: Vec<Uuid>,
    /// Live hosts in the whole subtree (UI 2.3 group card).
    pub host_count: u32,
    /// Direct children: subgroups plus hosts (non-empty group removal, UI 3.4).
    pub child_count: u32,
}

/// Result element of record queries and writes.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordView {
    Host(HostView),
    Group(GroupView),
}

impl RecordView {
    pub fn record_id(&self) -> Uuid {
        match self {
            Self::Host(view) => view.record_id,
            Self::Group(view) => view.record_id,
        }
    }
}

impl fmt::Debug for HostView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostView")
            .field("record_id", &self.record_id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for GroupView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupView")
            .field("record_id", &self.record_id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for RecordView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(view) => view.fmt(f),
            Self::Group(view) => view.fmt(f),
        }
    }
}

/// Input of `record_get` and `record_duplicate`; also a host link in stage 2 outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRef {
    pub vault_id: Uuid,
    pub record_id: Uuid,
}

/// Input of `record_delete` and `record_restore`.
#[derive(Clone, Debug, Deserialize)]
pub struct RecordsRef {
    pub vault_id: Uuid,
    pub record_ids: Vec<Uuid>,
}

/// Input of `record_save`. Rules in the plan, section "record_save".
#[derive(Deserialize)]
pub struct RecordSaveInput {
    pub vault_id: Uuid,
    /// `None` creates a new record (the draft became valid).
    #[serde(default)]
    pub record_id: Option<Uuid>,
    #[serde(rename = "type")]
    pub record_type: RecordType,
    /// The full data object without secret members. Start from the view's
    /// `data` and change only the edited members.
    pub data: Map<String, Value>,
    /// Secret members by dotted path: a string sets the value, `null` clears
    /// it, a missing path keeps the stored value.
    #[serde(default)]
    pub secrets: BTreeMap<String, Option<SecretString>>,
}

impl fmt::Debug for RecordSaveInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordSaveInput")
            .field("vault_id", &self.vault_id)
            .field("record_id", &self.record_id)
            .field("type", &self.record_type)
            .field("secret_paths", &self.secrets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Result of `record_delete`.
#[derive(Clone, Debug, Serialize)]
pub struct RecordDeleteResult {
    pub deleted: Vec<Uuid>,
    /// Children that a group removal moved one level up (docs/data-model.md, section 7.1).
    pub moved: Vec<Uuid>,
}

/// Where moved records go inside the target container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MovePosition {
    #[default]
    End,
    Before {
        record_id: Uuid,
    },
    After {
        record_id: Uuid,
    },
}

/// Input of `record_move`: drop on a group card, Move to, drag reorder.
#[derive(Clone, Debug, Deserialize)]
pub struct RecordMoveInput {
    pub vault_id: Uuid,
    pub record_ids: Vec<Uuid>,
    /// `None` moves to the top level.
    #[serde(default)]
    pub target_group_id: Option<Uuid>,
    #[serde(default)]
    pub position: MovePosition,
}

/// Input of `secret_reveal`.
#[derive(Clone, Debug, Deserialize)]
pub struct SecretFieldRef {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    /// A secret path of the record type, for example `ssh.password`.
    pub path: String,
}

/// Result of `secret_reveal`.
#[derive(Serialize)]
pub struct SecretValue {
    #[serde(serialize_with = "expose")]
    pub value: SecretString,
}

/// Input of `secret_copy`: what Rust copies to the clipboard (cleared after 30 s).
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretCopyTarget {
    RecordField {
        vault_id: Uuid,
        record_id: Uuid,
        path: String,
    },
    /// Allowed only within 5 minutes after `first_run_setup`,
    /// `recovery_key_show` or `recovery_key_reissue`, and never after a lock.
    RecoveryKey,
}

redacted_debug!(SecretValue);

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Every known setting key with its effective value (defaults applied),
/// for example `{"security.auto_lock_minutes": 15}` (docs/data-model.md, section 16.1).
pub type SettingsValues = BTreeMap<String, Value>;

/// Input of `settings_set`. Keys are validated; an unknown key or a bad value
/// refuses the whole call with `validation_failed` (field = the key).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SettingsSetInput {
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    /// Keys to reset to their defaults (their rows are deleted).
    #[serde(default)]
    pub reset: Vec<String>,
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSourceKind {
    Sftp,
    Session,
    Forwarding,
    Sync,
    Update,
    App,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSource {
    pub kind: NotificationSourceKind,
    /// The tab or record concerned.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// One notification (docs/data-model.md, section 16.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub id: u64,
    pub level: NotificationLevel,
    /// i18n key, for example `sftp.upload.done`.
    pub message_key: String,
    /// Values for the message; never secrets.
    pub params: BTreeMap<String, String>,
    pub source: NotificationSource,
    pub created_at: String,
    pub read: bool,
}

/// A notification to add (`Core::notify`).
#[derive(Clone, Debug, Deserialize)]
pub struct NewNotification {
    pub level: NotificationLevel,
    pub message_key: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    pub source: NotificationSource,
}

/// `notifications_list` and `notifications_mark_read`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NotificationList {
    /// Newest first, at most 200.
    pub items: Vec<Notification>,
    pub unread: u32,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// A broadcast event with its state version (docs/architecture.md, section 3.5).
#[derive(Clone, Debug)]
pub struct CoreEvent {
    /// Increases with every broadcast event during the life of the process.
    pub v: u64,
    pub kind: CoreEventKind,
}

/// Broadcast events of stage 1.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum CoreEventKind {
    /// `lock:changed`
    LockChanged { locked: bool, state: LockState },
    /// `records:changed`
    RecordsChanged {
        vault_id: Uuid,
        record_ids: Vec<Uuid>,
        classes: Vec<u8>,
    },
    /// `vaults:changed`
    VaultsChanged { vault_ids: Vec<Uuid> },
    /// `settings:changed`
    SettingsChanged { keys: Vec<String> },
    /// `notifications:changed`
    NotificationsChanged {
        unread: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        latest: Option<Notification>,
    },
    /// `sync:changed` (stage 7): the account or sync state changed; the
    /// interface reads `sync_status`.
    SyncChanged {},
}

impl CoreEventKind {
    /// Tauri event name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::LockChanged { .. } => "lock:changed",
            Self::RecordsChanged { .. } => "records:changed",
            Self::VaultsChanged { .. } => "vaults:changed",
            Self::SettingsChanged { .. } => "settings:changed",
            Self::NotificationsChanged { .. } => "notifications:changed",
            Self::SyncChanged {} => "sync:changed",
        }
    }
}

impl CoreEvent {
    /// Tauri event name.
    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    /// The event payload: the variant's fields plus `v`.
    pub fn payload(&self) -> Value {
        let mut payload = match serde_json::to_value(&self.kind) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        payload.insert("v".to_owned(), Value::from(self.v));
        Value::Object(payload)
    }
}

// ---------------------------------------------------------------------------
// Stage 2: sessions, local terminal, known hosts, keys, history, recent
// connections, windows (stage 2 plan, section 5). Written by the lead; owner
// S4 (core side) together with S5 (desktop side). `apps/desktop/src/ipc/types.ts`
// mirrors every type below.
// ---------------------------------------------------------------------------

/// What an SSH session connects to (docs/architecture.md, section 4.2, step 1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionTarget {
    /// A saved host, connected with its effective settings (docs/data-model.md, section 8).
    Host { vault_id: Uuid, record_id: Uuid },
    /// Quick connect from the search bar without a saved host (UI 2.1).
    Quick {
        address: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
}

impl fmt::Debug for SessionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host { record_id, .. } => f
                .debug_struct("Host")
                .field("record_id", record_id)
                .finish_non_exhaustive(),
            Self::Quick { .. } => f.write_str("Quick { .. }"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Ssh,
    Local,
    /// An SFTP session of the SFTP tab (stage 4): no terminal.
    Sftp,
    /// A Telnet session to a saved host's Telnet block (stage 9, D87).
    Telnet,
    /// A serial port of this device (stage 9, D88).
    Serial,
}

/// Session lifecycle (UI 4.6, 4.8; stage 2 plan, section 5.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// The first attempt, a Retry or a reconnect attempt is in progress (`step` set).
    Connecting,
    /// The shell is running.
    Connected,
    /// The shell ended with a non-zero exit status or a signal (`exit_status`);
    /// output stays, no automatic reconnect.
    Ended,
    /// Transport error, keepalive timeout or close without exit status
    /// (`error`); autoreconnect may run (`reconnect`).
    Dropped,
    /// The first attempt (or a Retry) failed (`error`, maybe `host_key`).
    Failed,
    /// Exit status 0 or closed by the user: the tab closes. Final.
    Closed,
}

/// Connection steps (UI 4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectStep {
    Resolving,
    Connecting,
    HostKey,
    Authenticating,
    OpeningSession,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Linux,
    Windows,
    Macos,
    Bsd,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsSource {
    Banner,
    Script,
}

/// OS detection result, the `host.os` member (docs/data-model.md, section 7.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostOs {
    pub family: OsFamily,
    /// Lowercase `ID` from `/etc/os-release`, or `freebsd`, `openbsd`, `netbsd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub source: OsSource,
    pub detected_at: String,
}

/// What Backspace sends (host `backspace`, UI 3.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backspace {
    #[default]
    Default,
    Del,
    CtrlH,
}

/// Why a session failed or dropped: an error code with its text `errors.<code>`
/// and, for `ssh_error`, the reason with its text `ssh_reason.<reason>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Autoreconnect state of a dropped session (UI 4.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReconnectInfo {
    /// 1-based number of the next (or running) attempt.
    pub attempt: u32,
    /// Milliseconds until the next attempt, measured when the event was
    /// emitted; absent while an attempt runs or while waiting for unlock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_in_ms: Option<u64>,
    /// Locked: attempts wait for unlock (UI 13.3).
    pub waiting_for_unlock: bool,
    /// An attempt is running now.
    pub in_progress: bool,
}

/// A presented host key, for the dialogs of UI 10.2.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct HostKeyInfo {
    pub address: String,
    pub port: u16,
    pub key_type: String,
    /// `SHA256:` fingerprint.
    pub fingerprint: String,
}

/// A pending question of a session (UI 4.8, 10.2). Replaced or removed by the
/// next `sessions:changed` of the session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionPrompt {
    /// Unique within the session; `session_prompt_reply` names it.
    pub prompt_id: u64,
    #[serde(flatten)]
    pub kind: PromptKind,
}

/// Prompt kinds. Server texts (`name`, `instruction`, `prompt`) are untrusted.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptKind {
    /// First connection (TOFU): Accept and save / Connect once / Cancel.
    HostKeyUnknown {
        key: HostKeyInfo,
    },
    /// The saved key differs: only Replace saved key continues.
    HostKeyChanged {
        key: HostKeyInfo,
        old_key_type: String,
        old_fingerprint: String,
    },
    Username,
    Password {
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        /// The previous password was refused.
        error: bool,
        /// Show the "Save" checkbox.
        can_save: bool,
    },
    Passphrase {
        #[serde(skip_serializing_if = "Option::is_none")]
        key_label: Option<String>,
        error: bool,
        can_save: bool,
    },
    KeyboardInteractive {
        name: String,
        instruction: String,
        prompt: String,
        /// `false`: mask the answer.
        echo: bool,
        index: u32,
        count: u32,
        error: bool,
    },
}

/// A session as its window sees it: the payload of `sessions:changed` and
/// the elements of `StateSnapshot.sessions`.
#[derive(Clone, Serialize)]
pub struct SessionInfo {
    pub session_id: Uuid,
    /// Increases with every change of this session. A window applies an
    /// event only when its `rev` is greater than the one it has.
    pub rev: u64,
    pub kind: SessionKind,
    /// Label of the owning window (`w-<n>`).
    pub window: String,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<ConnectStep>,
    /// Tab title: the host label or address; the shell label for a local session.
    pub title: String,
    /// SSH sessions: what to connect to again (Duplicate, Retry, Restore).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SessionTarget>,
    /// Local sessions: the shell id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// The host's detected OS (tab icon).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<HostOs>,
    /// Effective terminal theme, a `theme_ref` (docs/data-model.md, section 8.5).
    pub terminal_theme: Value,
    pub backspace: Backspace,
    /// Effective "Record history": the UI sends history candidates only when true.
    pub record_history: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<SessionPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SessionError>,
    /// The presented key when the error is `host_key_revoked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key: Option<HostKeyInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconnect: Option<ReconnectInfo>,
}

impl fmt::Debug for SessionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionInfo")
            .field("session_id", &self.session_id)
            .field("rev", &self.rev)
            .field("kind", &self.kind)
            .field("state", &self.state)
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

/// Input of `session_open`.
#[derive(Clone, Debug, Deserialize)]
pub struct SessionOpenInput {
    pub target: SessionTarget,
    pub cols: u16,
    pub rows: u16,
}

/// Input of `local_open`.
#[derive(Clone, Debug, Deserialize)]
pub struct LocalOpenInput {
    /// An id from `local_shells`; absent = the `terminal.local_shell` setting or the default.
    #[serde(default)]
    pub shell_id: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// Input of `session_cancel`, `session_close` and `session_reconnect`.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct SessionRef {
    pub session_id: Uuid,
}

/// A command line typed by the user, captured at Enter by the UI
/// (docs/security.md, section 10.1, rules 1-3). The core applies rule 4 and
/// the secret detector before anything is stored.
#[derive(Clone, Deserialize)]
pub struct HistoryCandidate {
    /// The logical line from the prompt boundary to its end, trailing spaces removed.
    pub command: String,
    /// Text before the boundary on the same line, or the last non-empty
    /// line above when that is empty (password prompt check). Never stored.
    #[serde(default)]
    pub prompt_text: String,
    /// The user typed at least one printable character after the boundary.
    #[serde(default)]
    pub typed_printable: bool,
    /// The alternate screen was active when typing started or at Enter.
    #[serde(default)]
    pub alt_screen: bool,
}

/// Input of `session_write`: typed input (and pastes). The write that
/// carries the Enter of a command line may carry its history candidate.
#[derive(Clone, Deserialize)]
pub struct SessionWriteInput {
    pub session_id: Uuid,
    pub data: String,
    /// `true`: every UTF-16 code unit of `data` is one byte (xterm.js `onBinary`).
    #[serde(default)]
    pub binary: bool,
    #[serde(default)]
    pub history: Option<HistoryCandidate>,
}

/// Input of `session_inject`: input the app sends on its own (a history
/// entry insert in stage 2; snippets from stage 5). Never recorded in history.
#[derive(Clone, Deserialize)]
pub struct SessionInjectInput {
    pub session_id: Uuid,
    pub data: String,
}

/// Input of `session_resize`.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct SessionResizeInput {
    pub session_id: Uuid,
    pub cols: u16,
    pub rows: u16,
}

/// Decision in the host key dialogs (UI 10.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyDecision {
    /// Unknown key: save a `known_host` record (`source = tofu`) and continue.
    AcceptSave,
    /// Unknown key: continue without saving.
    ConnectOnce,
    /// Changed key: update the matching record (`source = replaced`) and continue.
    Replace,
    Cancel,
}

/// Answer to a [`SessionPrompt`].
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptReply {
    HostKey {
        decision: HostKeyDecision,
    },
    /// Username, password, passphrase or keyboard-interactive answer.
    Text {
        value: SecretString,
        /// The "Save" checkbox (password and passphrase only).
        #[serde(default)]
        save: bool,
    },
    Cancel,
}

/// Input of `session_prompt_reply`.
#[derive(Deserialize)]
pub struct SessionPromptReplyInput {
    pub session_id: Uuid,
    pub prompt_id: u64,
    pub reply: PromptReply,
}

redacted_debug!(
    HistoryCandidate,
    SessionWriteInput,
    SessionInjectInput,
    PromptReply,
    SessionPromptReplyInput
);

/// A local shell for `local_open` (UI 12.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LocalShell {
    pub id: String,
    pub label: String,
}

/// Result of `local_shells`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LocalShellsResult {
    pub shells: Vec<LocalShell>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownHostSource {
    Tofu,
    Replaced,
    Import,
}

/// A `known_host` record as the Known Hosts screen sees it (UI 10.1).
#[derive(Clone, Serialize)]
pub struct KnownHostView {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    /// "Date added": the UUIDv7 time of the record id.
    pub created_at: String,
    pub modified_at: String,
    /// The stored `host`: a name, an address, a pattern or `|1|<salt>|<hash>`.
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub hashed: bool,
    /// `host` for port 22, `[host]:port` otherwise; absent for hashed entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_address: Option<String>,
    pub key_type: String,
    pub fingerprint: String,
    pub revoked: bool,
    pub source: KnownHostSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Saved hosts whose address and effective port this entry matches.
    pub host_ids: Vec<Uuid>,
}

impl fmt::Debug for KnownHostView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KnownHostView")
            .field("record_id", &self.record_id)
            .finish_non_exhaustive()
    }
}

/// Input of the e2e mock action `known_host_add` and of the stage 3
/// known_hosts import: one entry as it would come from a known_hosts line.
#[derive(Clone, Deserialize)]
pub struct KnownHostAddInput {
    /// A host name or address, a pattern, or `|1|<salt>|<hash>`.
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub key_type: String,
    /// Base64 key blob.
    pub public_key: String,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub comment: Option<String>,
}

redacted_debug!(KnownHostAddInput);

/// The summary of a known_hosts import (UI 10.1): four numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct KnownHostsImportSummary {
    /// New entries (one per host pattern).
    pub imported: usize,
    /// Entries equal to one already in the list (natural key, data-model
    /// 18.4), or to an earlier one of the same file.
    pub duplicates: usize,
    /// `@cert-authority` lines and negated (`!`) host patterns.
    pub skipped: usize,
    /// Lines that could not be read.
    pub invalid: usize,
}

/// A `key` record without its secrets (docs/security.md, section 9.5).
#[derive(Clone, Serialize)]
pub struct KeyView {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub created_at: String,
    pub modified_at: String,
    pub label: String,
    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`, ..., or `unknown`.
    pub algorithm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The private key is protected by a passphrase.
    pub encrypted: bool,
    /// The passphrase is saved in the vault.
    pub passphrase_saved: bool,
}

impl fmt::Debug for KeyView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyView")
            .field("record_id", &self.record_id)
            .field("algorithm", &self.algorithm)
            .finish_non_exhaustive()
    }
}

/// Input of `key_import`: the short "+ -> Key" path of the host panel (UI 3.1, D3).
#[derive(Deserialize)]
pub struct KeyImportInput {
    pub vault_id: Uuid,
    /// Default: the key comment, else the algorithm name.
    #[serde(default)]
    pub label: Option<String>,
    /// OpenSSH private key text (pasted or read from a chosen file).
    pub private_key: SecretString,
    #[serde(default)]
    pub passphrase: Option<SecretString>,
    /// Store the passphrase in the vault (UI 7.3); otherwise it is asked on every connection.
    #[serde(default)]
    pub save_passphrase: bool,
}

redacted_debug!(KeyImportInput);

/// Input of `key_generate` (UI 7.1 "Generate key"; night decision D32).
#[derive(Clone, Deserialize)]
pub struct KeyGenerateInput {
    pub vault_id: Uuid,
    /// Required (UI 7.3); also the key comment.
    pub label: String,
    #[serde(default)]
    pub kind: crate::keys::generate::KeyKind,
    /// Optional: the new key is encrypted with it.
    #[serde(default)]
    pub passphrase: Option<SecretString>,
    /// Store the passphrase in the vault (UI 7.3); otherwise it is asked on every connection.
    #[serde(default)]
    pub save_passphrase: bool,
}

redacted_debug!(KeyGenerateInput);

/// Input of `data_export` (SPEC principle 4; docs/storage-format.md,
/// sections 13.2 and 13.3; night decision D34).
#[derive(Clone, Default, Deserialize)]
pub struct DataExportInput {
    /// `Some`: the encrypted export with this password (at least 8
    /// characters, as the master password); `None`: the plain export, whose
    /// secrets are in clear text (the UI warns first).
    #[serde(default)]
    pub password: Option<SecretString>,
    /// Write the command history (and the hosts' dismissed history hints)
    /// too; off by default (D50).
    #[serde(default)]
    pub include_history: bool,
    /// Write the connection log too; off by default (D50).
    #[serde(default)]
    pub include_connection_log: bool,
    /// Stage 7 (UI 14, "Export…"): when not empty, the file holds only
    /// these entries of Recently deleted (their last live versions) and no
    /// settings; the two options above do not apply.
    #[serde(default)]
    pub recently_deleted: Vec<Uuid>,
}

redacted_debug!(DataExportInput);

/// Result of `data_export`: the file content and what it holds.
pub struct DataExport {
    /// UTF-8 JSON, LF line endings. Holds secrets in the plain format.
    pub text: SecretString,
    pub encrypted: bool,
    /// Exported records by type name.
    pub counts: BTreeMap<String, usize>,
    /// The suggested file name (`shellpp-export-<UTC>.json` or
    /// `.encrypted.json`, docs/storage-format.md, section 13.1).
    pub file_name: String,
}

impl std::fmt::Debug for DataExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataExport")
            .field("encrypted", &self.encrypted)
            .field("counts", &self.counts)
            .finish_non_exhaustive()
    }
}

/// Input of `data_import` (night decision D34).
#[derive(Clone, Deserialize)]
pub struct DataImportInput {
    /// The file content (a plain or an encrypted export).
    pub text: SecretString,
    /// The password of an encrypted export.
    #[serde(default)]
    pub password: Option<SecretString>,
    /// Apply the app settings the export holds (the default).
    #[serde(default = "default_true")]
    pub apply_settings: bool,
    /// Import the command history the file holds (the default, D51): each
    /// entry goes through the secret detector of live recording.
    #[serde(default = "default_true")]
    pub import_history: bool,
    /// Import the connection log the file holds (the default, D51).
    #[serde(default = "default_true")]
    pub import_connection_log: bool,
    /// "Full restore from my backup" (D50): every setting of the file is
    /// applied, also those D41 keeps local. Off by default; implies
    /// `apply_settings`.
    #[serde(default)]
    pub full_restore: bool,
}

redacted_debug!(DataImportInput);

impl DataImportInput {
    /// An import of `text` with the default options: the file's settings
    /// applied by D41, its history and connection log imported, no full
    /// restore.
    pub fn new(text: SecretString, password: Option<SecretString>) -> Self {
        Self {
            text,
            password,
            apply_settings: true,
            import_history: true,
            import_connection_log: true,
            full_restore: false,
        }
    }
}

/// Input of `data_import_contents`: the file and, for an encrypted one, its
/// password.
#[derive(Clone, Deserialize)]
pub struct DataImportContentsInput {
    pub text: SecretString,
    #[serde(default)]
    pub password: Option<SecretString>,
}

redacted_debug!(DataImportContentsInput);

/// What an export file holds that Import Data asks about (D51).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DataImportContents {
    /// Command history entries.
    pub history: usize,
    /// Connection log entries.
    pub connection_log: usize,
}

fn default_true() -> bool {
    true
}

/// Where an `~/.ssh/config` import reads (D35): the file and the home
/// folder for `~` and `%d`. Tests and the demo pass temporary paths.
#[derive(Clone, Debug, Deserialize)]
pub struct SshConfigSource {
    pub path: std::path::PathBuf,
    pub home: std::path::PathBuf,
}

/// The state of one entry of an import preview (UI 2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    New,
    Exists,
    Error,
}

/// One host of an `~/.ssh/config` preview.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SshConfigHostPreview {
    /// The first alias of its `Host` line; also the label.
    pub alias: String,
    pub address: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub status: ImportStatus,
    /// Why it is not imported (`status = error`), for example ProxyCommand.
    pub reason: Option<String>,
    /// Options of this host that are not imported, and why.
    pub notes: Vec<String>,
    /// Its `IdentityFile` paths (after `~` and `%d`).
    pub key_files: Vec<String>,
    /// Its `ProxyJump` hops as written.
    pub jump: Vec<String>,
}

/// The state of a key file of an `~/.ssh/config` preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "reason")]
pub enum KeyFileStatus {
    /// A key that the import adds to the Keychain.
    New,
    /// The Keychain holds this key (same fingerprint).
    Exists,
    /// Encrypted: the import needs its passphrase.
    NeedsPassphrase,
    /// The file does not exist.
    Missing,
    /// Not a usable key, with the reason.
    Invalid(String),
}

/// One key file of an `~/.ssh/config` preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SshConfigKeyPreview {
    pub path: String,
    pub status: KeyFileStatus,
}

/// The preview of an `~/.ssh/config` import (UI 2.5 step 3; D35).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SshConfigPreview {
    pub hosts: Vec<SshConfigHostPreview>,
    pub keys: Vec<SshConfigKeyPreview>,
    /// Lines that could not be read (file, line, what).
    pub problems: Vec<String>,
    /// `Host` lines with only patterns (templates): applied, not imported.
    pub templates: usize,
}

/// Input of `ssh_config_import`.
#[derive(Clone, Deserialize)]
pub struct SshConfigImportInput {
    pub source: SshConfigSource,
    /// The aliases chosen in the preview; `None` = every new host.
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
    /// Passphrases of encrypted key files, by path as the preview shows it.
    #[serde(default)]
    pub passphrases: BTreeMap<String, SecretString>,
    /// Store those passphrases in the vault.
    #[serde(default)]
    pub save_passphrases: bool,
}

redacted_debug!(SshConfigImportInput);

/// Input of `putty_import` (run 8, D140). The preview and the summary of a
/// PuTTY import are those of an `~/.ssh/config` import: a session is a
/// host whose alias is the session name, its `PublicKeyFile` a key file.
#[derive(Clone, Default, Deserialize)]
pub struct PuttyImportInput {
    /// The session names chosen in the preview; `None` = every new one.
    #[serde(default)]
    pub sessions: Option<Vec<String>>,
    /// Passphrases of encrypted key files, by path as the preview shows it.
    #[serde(default)]
    pub passphrases: BTreeMap<String, SecretString>,
    /// Store those passphrases in the vault.
    #[serde(default)]
    pub save_passphrases: bool,
}

redacted_debug!(PuttyImportInput);

/// A CSV file and how to read it (run 8, D141): the input of
/// `csv_inspect`, `csv_preview` and `csv_import`. Each setting left out is
/// detected (`csv_inspect` returns what was chosen). The bytes are wiped
/// when dropped: the file may hold passwords.
#[derive(Clone, Default)]
pub struct CsvInput {
    pub bytes: zeroize::Zeroizing<Vec<u8>>,
    /// A WHATWG label (`crate::import::csv::ENCODINGS` are offered).
    pub encoding: Option<String>,
    pub delimiter: Option<char>,
    /// The first row names the columns.
    pub has_header: Option<bool>,
    /// What each column holds; at most one column per role but
    /// `ignore`. `address` is required.
    pub roles: Option<Vec<crate::import::csv::Role>>,
}

redacted_debug!(CsvInput);

/// What `csv_inspect` found: the settings in effect and the columns.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvInspection {
    pub encoding: String,
    pub delimiter: char,
    pub has_header: bool,
    /// The header's names, or "Column N".
    pub columns: Vec<String>,
    pub roles: Vec<crate::import::csv::Role>,
    /// The first data rows as they are (a password column too: the dialog
    /// masks the cells of the column mapped to `password`).
    pub sample: Vec<Vec<String>>,
    /// Data rows in the file.
    pub rows: usize,
}

redacted_debug!(CsvInspection);

/// One row of a CSV preview.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvRowPreview {
    /// The record's number in the file (the header is 1).
    pub line: usize,
    pub label: Option<String>,
    pub address: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// The group path, ` / ` between levels.
    pub group: Option<String>,
    pub status: ImportStatus,
    /// Why it is not imported (`status = error`).
    pub reason: Option<String>,
}

redacted_debug!(CsvRowPreview);

/// The preview of a CSV import (UI 2.5 step 3; D141).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CsvPreview {
    pub rows: Vec<CsvRowPreview>,
    /// Groups the import creates.
    pub groups_new: usize,
    /// Rows that carry a password (the dialog warns).
    pub passwords: usize,
}

/// Result of `csv_import`: "added N, skipped M, errors K" (UI 2.5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CsvImportSummary {
    pub added: usize,
    /// Rows that match a host (or an earlier row).
    pub skipped: usize,
    /// Rows not imported: (record number, reason).
    pub errors: Vec<(usize, String)>,
    pub groups_added: usize,
    /// Rows that carried a password.
    pub passwords: usize,
}

/// Result of `ssh_config_import`: "added N, skipped M, errors K" (UI 2.5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SshConfigImportSummary {
    pub added: usize,
    /// Hosts imported before (same `import_key`).
    pub skipped: usize,
    /// Hosts not imported, with the reason.
    pub errors: Vec<(String, String)>,
    pub keys_added: usize,
    pub keys_existing: usize,
    /// Key files left out (missing, invalid, no passphrase given).
    pub keys_skipped: usize,
    /// Imported hosts whose `ProxyJump` names a host that was not imported:
    /// the chain keeps the missing hop and the host is not connected (their
    /// notes say so).
    pub jump_chains_left_out: usize,
    /// Commands of the file that were neither run nor stored
    /// (`ProxyCommand`, `LocalCommand`, `PermitLocalCommand`,
    /// `KnownHostsCommand`, `Match exec`; security.md 13.5).
    pub commands_skipped: usize,
    /// Forwarding rules made from `LocalForward`, `RemoteForward` and
    /// `DynamicForward` of the imported hosts (stage 5).
    pub rules_added: usize,
}

/// One record of an import that was not imported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DataImportRefusal {
    pub record_id: Uuid,
    #[serde(rename = "type")]
    pub record_type: String,
    pub errors: Vec<crate::error::FieldError>,
}

/// Result of `data_import`: "added N, skipped M, errors K" (UI 2.5) and the
/// details.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DataImportSummary {
    /// New records (under their exported ids).
    pub added: usize,
    /// Records that already exist with the same content (same id, or the
    /// same key fingerprint or known host).
    pub existing: usize,
    /// Records whose id exists here with other content: imported as a copy
    /// with a new id and an " (imported)" suffix; the local one stays.
    pub conflicts: usize,
    /// Records of types this import does not take (device records, the
    /// connection log, history, types of later versions).
    pub skipped: usize,
    /// Records refused with their errors.
    pub refused: Vec<DataImportRefusal>,
    /// App settings applied from the export.
    pub settings: usize,
    /// Settings of the file an import never applies (commands, programs,
    /// URLs and security settings, D41) whose value differs from the one
    /// here.
    pub settings_left_out: usize,
    /// Keys this client cannot read, kept as "unknown" records so that
    /// nothing of the file is lost (D50; counted in `added` or `conflicts`
    /// too).
    pub unknown_keys: usize,
    /// History entries left out because the secret detector or the capture
    /// rules of live recording would not have recorded them (D51).
    pub history_left_out: usize,
    /// Forwarding rules whose autostart the import turned off: on launch,
    /// and on connect beyond loopback (D63; not in a full restore).
    pub rules_autostart_off: usize,
}

/// Result of `key_import`.
#[derive(Clone, Debug, Serialize)]
pub struct KeyImportResult {
    pub key: KeyView,
    /// `false`: a key with the same fingerprint existed and is returned instead.
    pub created: bool,
}

/// Input of `history_list`.
#[derive(Clone, Default, Deserialize)]
pub struct HistoryListInput {
    /// The host whose history to list; absent = entries without a host
    /// (local terminals and quick connects).
    #[serde(default)]
    pub host: Option<RecordRef>,
    /// Case-insensitive substring filter.
    #[serde(default)]
    pub text: Option<String>,
    /// Default 500, at most 5000.
    #[serde(default)]
    pub limit: Option<u32>,
}

impl fmt::Debug for HistoryListInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryListInput")
            .field("host", &self.host)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

/// One history entry, newest first in lists.
#[derive(Clone, Serialize)]
pub struct HistoryEntryView {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub command: String,
    pub executed_at: String,
    /// The saved host it ran on; `None` for local terminals and quick
    /// connects.
    pub host: Option<RecordRef>,
    /// `history_list_distinct`: the older entries of the same command that
    /// this one stands for (newest first); empty otherwise.
    pub older: Vec<Uuid>,
}

/// Input of `history_search` (UI 9.1, the Shell History screen: every
/// host, filtered by host and text).
#[derive(Clone, Default, Deserialize)]
pub struct HistorySearchInput {
    /// Only these hosts; empty = every entry (with or without a host).
    #[serde(default)]
    pub hosts: Vec<RecordRef>,
    /// Case-insensitive substring filter.
    #[serde(default)]
    pub text: Option<String>,
    /// Default 500, at most 5000.
    #[serde(default)]
    pub limit: Option<u32>,
}

impl fmt::Debug for HistorySearchInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistorySearchInput")
            .field("hosts", &self.hosts.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for HistoryEntryView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryEntryView")
            .field("record_id", &self.record_id)
            .finish_non_exhaustive()
    }
}

/// Input of `history_clear`: every entry of one host (or of no host).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct HistoryClearInput {
    #[serde(default)]
    pub host: Option<RecordRef>,
}

/// Result of `history_clear`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryClearResult {
    pub removed: u32,
}

/// The "save as snippets" hint of a host's History tab (UI 4.2;
/// docs/data-model.md, sections 13.1 and 13.2).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HistoryHint {
    /// At most three commands: the most frequent first (live entries of the
    /// host), ties by the newest use. Commands that look like secrets
    /// (docs/security.md, section 10.2) are never offered.
    pub commands: Vec<String>,
    /// "Close" was chosen for this host (`history_hint_state.dismissed`).
    pub dismissed: bool,
}

impl fmt::Debug for HistoryHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryHint")
            .field("commands", &self.commands.len())
            .field("dismissed", &self.dismissed)
            .finish()
    }
}

/// A row of the Logs screen (UI 11.1; docs/data-model.md, section 13.3).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionLogView {
    pub record_id: Uuid,
    pub started_at: String,
    /// Absent while the session is active, or when its end is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// `closed`, `exit_status`, `dropped` or `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<u32>,
    pub protocol: String,
    /// Absent for a quick connect without a saved host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<RecordRef>,
    /// The host title now (label or address), else the snapshot.
    pub title: String,
    pub address: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<HostOs>,
    pub saved: bool,
    /// The session ran on this device.
    pub this_device: bool,
    /// Its output was recorded (the file may be on another device).
    pub recorded: bool,
}

impl ConnectionLogView {
    /// No end yet (UI 11.1 shows "now").
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none() && self.end_reason.is_none()
    }
}

impl fmt::Debug for ConnectionLogView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionLogView")
            .field("record_id", &self.record_id)
            .field("saved", &self.saved)
            .finish_non_exhaustive()
    }
}

/// A row of Recent connections (UI 6; docs/data-model.md, section 13.3).
#[derive(Clone, Serialize)]
pub struct RecentConnection {
    /// Absent for a quick connect without a saved host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<RecordRef>,
    /// The host title now (label or address), else the snapshot.
    pub title: String,
    pub address: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<HostOs>,
    /// "vault / group": the vault name, then the group names from the top level.
    pub path: Vec<String>,
    /// Start of the latest session.
    pub last_at: String,
}

impl fmt::Debug for RecentConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecentConnection")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// What a window opened by `window_open` does first (UI 2.4 with Alt, UI 4.4
/// "Duplicate in a new window"). Delivered once in its first `state_snapshot`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowIntent {
    OpenSession {
        target: SessionTarget,
    },
    OpenLocal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell_id: Option<String>,
    },
}

/// Input of `window_open` (optional).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WindowOpenInput {
    #[serde(default)]
    pub intent: Option<WindowIntent>,
}

/// Input of `app_exit` and `window_close` (optional).
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ExitInput {
    /// The user confirmed closing active sessions (UI 4.6).
    #[serde(default)]
    pub confirmed: bool,
}

/// Result of `app_exit` and `window_close`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ExitResult {
    /// Nothing was closed: active sessions need the user's confirmation.
    pub needs_confirmation: bool,
    /// Active sessions (app-wide for `app_exit`, of the window for `window_close`).
    pub active_sessions: u32,
}

/// Debug without host data or server text (docs/security.md, section 9.1).
impl std::fmt::Debug for HostKeyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostKeyInfo")
            .field("port", &self.port)
            .field("key_type", &self.key_type)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// Debug without host data or server text (docs/security.md, section 9.1).
impl std::fmt::Debug for SshConfigHostPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConfigHostPreview")
            .finish_non_exhaustive()
    }
}

/// Debug with the variant only: no user names or server text
/// (docs/security.md, section 9.1).
impl std::fmt::Debug for PromptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let variant = match self {
            Self::HostKeyUnknown { .. } => "HostKeyUnknown",
            Self::HostKeyChanged { .. } => "HostKeyChanged",
            Self::Username => "Username",
            Self::Password { .. } => "Password",
            Self::Passphrase { .. } => "Passphrase",
            Self::KeyboardInteractive { .. } => "KeyboardInteractive",
        };
        f.debug_struct("PromptKind")
            .field("variant", &variant)
            .finish_non_exhaustive()
    }
}
