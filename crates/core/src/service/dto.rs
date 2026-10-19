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
//! Owner: W2 (contract frozen by the lead; see the plan before changing it).

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

/// Input of `record_get` and `record_duplicate`.
#[derive(Clone, Copy, Debug, Deserialize)]
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
