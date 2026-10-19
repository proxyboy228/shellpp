// IPC contract types: the JSON shapes exchanged with the Rust core
// (docs/architecture.md, sections 3.4 and 3.5).
//
// This file mirrors crates/core/src/service/dto.rs and the desktop-only types
// of apps/desktop/src-tauri/src/commands. Change both sides together and
// record the change in the stage plan.
//
// Conventions: snake_case field names; optional output fields are omitted
// (never null); UUIDs are lowercase hyphenated strings; timestamps are
// RFC 3339 UTC with milliseconds.

/** UUID text, lowercase and hyphenated. */
export type Uuid = string;
/** RFC 3339 UTC timestamp with milliseconds, for example `2026-10-18T22:31:05.123Z`. */
export type Timestamp = string;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/** Stable error codes (crates/core/src/error.rs). Each has the i18n key `errors.<code>`. */
export type ErrorCode =
  | 'locked'
  | 'wrong_state'
  | 'invalid_password'
  | 'invalid_recovery_key'
  | 'not_found'
  | 'validation_failed'
  | 'read_only'
  | 'too_large'
  | 'unavailable'
  | 'newer_schema'
  | 'damaged'
  | 'storage_error'
  | 'io_error'
  | 'internal';

export type ValidationReason =
  | 'required'
  | 'too_long'
  | 'too_short'
  | 'invalid'
  | 'out_of_range'
  | 'duplicate'
  | 'too_many'
  | 'cycle'
  | 'too_deep'
  | 'not_allowed'
  | 'missing_ref';

/** One refused field; `field` is a dotted path such as `ssh.port` or `tags.3`. */
export interface FieldError {
  field: string;
  reason: ValidationReason;
}

/** The error shape of every command: `{"code": "...", "details": {...}}`. */
export interface CommandErrorPayload {
  code: ErrorCode;
  /** `{errors: FieldError[]}` for `validation_failed`, `{found: number}` for `newer_schema`, else `{}`. */
  details: { errors?: FieldError[]; found?: number };
}

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

export type TagColor = 'graphite' | 'jade' | 'amber' | 'coral' | 'plum' | 'moss' | 'ocean' | 'rose';
export type VaultKind = 'personal' | 'team';
export type VaultStatus = 'active' | 'archived';
export type Role = 'owner' | 'editor' | 'viewer';
export type OsKind = 'windows' | 'macos' | 'linux' | 'ios' | 'android';
export type RecordType = 'host' | 'group';

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export interface AppInfo {
  /** Always "Shell++". */
  name: string;
  /** Semver of the app. */
  version: string;
  os: OsKind;
  arch: string;
  debug: boolean;
  /** Built with the `e2e-mocks` feature. */
  e2e: boolean;
}

export interface VaultView {
  vault_id: Uuid;
  kind: VaultKind;
  status: VaultStatus;
  role?: Role;
  name: string;
  color: TagColor;
}

/** `state_snapshot` (docs/architecture.md, section 3.5, snapshot protocol). */
export interface StateSnapshot {
  /** Apply only buffered events whose `v` is greater. */
  v: number;
  lock: LockStatus;
  settings: SettingsValues;
  notifications: NotificationList;
  /** Empty while locked or at first run. */
  vaults: VaultView[];
  /** Label of the calling window, `w-<n>`. */
  window_label: string;
}

export interface WindowOpenResult {
  label: string;
}

export interface ExternalOpenInput {
  /** `https:` only. */
  url: string;
}

// ---------------------------------------------------------------------------
// Lock
// ---------------------------------------------------------------------------

export type LockState = 'first_run' | 'locked' | 'unlocked';

export interface LockStatus {
  state: LockState;
  /** An OS keychain is usable; otherwise "Remember on this device" is hidden. */
  remember_device_available: boolean;
  remember_device_enabled: boolean;
}

export interface FirstRunInput {
  password: string;
}

export interface PasswordInput {
  password: string;
}

export interface RecoveryUnlockInput {
  recovery_key: string;
  new_password: string;
}

export interface PasswordChangeInput {
  current_password: string;
  new_password: string;
}

export interface RememberDeviceInput {
  enabled: boolean;
}

export interface RecoveryKeyResult {
  /** `RK1-XXXXX-XXXXX-...` (docs/security.md, section 5.8). */
  recovery_key: string;
}

// ---------------------------------------------------------------------------
// Record data (docs/data-model.md, sections 3.5, 7.1-7.3)
// ---------------------------------------------------------------------------

export type ThemeRef = { builtin: string } | { record_id: Uuid };

export interface EnvVar {
  name: string;
  value: string;
}

export interface HostProxy {
  type: 'http' | 'socks5';
  address: string;
  port: number;
  username?: string;
  // `password` is a secret member: path `ssh.proxy.password`.
}

export interface HostOs {
  family: 'linux' | 'windows' | 'macos' | 'bsd' | 'other';
  distro?: string;
  version?: string;
  source: 'banner' | 'script';
  detected_at: Timestamp;
}

/** `host.ssh` without secret members (`ssh.password` is a secret path). */
export interface HostSsh {
  port?: number;
  username?: string;
  identity_id?: Uuid;
  key_id?: Uuid;
  certificate_key_id?: Uuid;
  fido2_key_id?: Uuid;
  jump_chain?: Uuid[];
  proxy?: HostProxy;
  agent_forwarding?: boolean;
  startup_snippet_id?: Uuid;
  env?: EnvVar[];
  keepalive_s?: number;
  mosh?: boolean;
}

/**
 * The `host` data object as the webview sees it. Round-trip rule: start from
 * the view's `data`, change only the edited members, send the whole object.
 * `order_key`, `import_key` and `os` are managed by the core and ignored on input.
 */
export interface HostData {
  address: string;
  label?: string;
  group_id?: Uuid;
  tags?: string[];
  notes?: string;
  color?: TagColor;
  order_key?: string;
  backspace?: 'default' | 'del' | 'ctrl_h';
  charset?: string;
  terminal_theme?: ThemeRef;
  record_history?: 'default' | 'on' | 'off';
  record_output?: 'default' | 'on' | 'off';
  primary_protocol?: 'ssh' | 'telnet';
  ssh?: HostSsh;
  telnet?: { port?: number };
  os?: HostOs;
  import_key?: string;
  /** Unknown members are kept unchanged. */
  [member: string]: unknown;
}

export interface GroupSsh {
  port?: number;
  username?: string;
  identity_id?: Uuid;
  key_id?: Uuid;
  jump_chain?: Uuid[];
}

/** The `group` data object. `order_key` and `import_key` are managed by the core. */
export interface GroupData {
  name: string;
  parent_id?: Uuid;
  color?: TagColor;
  notes?: string;
  order_key?: string;
  ssh?: GroupSsh;
  import_key?: string;
  [member: string]: unknown;
}

/** Secret member paths per type in stage 1. */
export const SECRET_PATHS = {
  host: ['ssh.password', 'ssh.proxy.password'],
  group: [],
} as const satisfies Record<RecordType, readonly string[]>;

export type HostSecretPath = (typeof SECRET_PATHS)['host'][number];

// ---------------------------------------------------------------------------
// Record views and commands
// ---------------------------------------------------------------------------

export interface Inherited<T> {
  value: T;
  /** The group whose `ssh` block supplied the value ("from group X"). */
  from_group_id: Uuid;
}

/** Values inherited from ancestor groups, ignoring the record's own level. */
export interface InheritedValues {
  port?: Inherited<number>;
  username?: Inherited<string>;
  identity_id?: Inherited<Uuid>;
  key_id?: Inherited<Uuid>;
  jump_chain?: Inherited<Uuid[]>;
}

interface RecordViewBase {
  vault_id: Uuid;
  record_id: Uuid;
  revision: number;
  created_at: Timestamp;
  modified_at: Timestamp;
  inherited: InheritedValues;
  /** Ancestor group ids, top level first (for a host: ending with its own group). */
  path: Uuid[];
}

export interface HostView extends RecordViewBase {
  type: 'host';
  data: HostData;
  /** Secret paths that hold a value, for example `['ssh.password']`. */
  secrets_set: HostSecretPath[];
}

export interface GroupView extends RecordViewBase {
  type: 'group';
  data: GroupData;
  /** Live hosts in the whole subtree. */
  host_count: number;
  /** Direct subgroups plus hosts. */
  child_count: number;
}

export type RecordView = HostView | GroupView;

export type QueryScope = { kind: 'all' } | { kind: 'top_level' } | { kind: 'group'; group_id: Uuid };

export type SortMode = 'manual' | 'name' | 'created' | 'modified';

export interface RecordsQuery {
  /** Default: the personal vault. */
  vault_id?: Uuid;
  types: RecordType[];
  /** Default `{kind: 'all'}`. */
  scope?: QueryScope;
  /** Case-insensitive substring search (plan, "records_query"). */
  text?: string;
  /** Hosts must carry every tag (case-insensitive). */
  tags?: string[];
  /** Default `'manual'`. */
  sort?: SortMode;
}

export interface RecordRef {
  vault_id: Uuid;
  record_id: Uuid;
}

export interface RecordsRef {
  vault_id: Uuid;
  record_ids: Uuid[];
}

interface RecordSaveBase {
  vault_id: Uuid;
  /** Omit or null to create the record (the draft became valid). */
  record_id?: Uuid | null;
}

export interface HostSaveInput extends RecordSaveBase {
  type: 'host';
  data: HostData;
  /** Secret members: a string sets, null clears, a missing path keeps the stored value. */
  secrets?: Partial<Record<HostSecretPath, string | null>>;
}

export interface GroupSaveInput extends RecordSaveBase {
  type: 'group';
  data: GroupData;
}

export type RecordSaveInput = HostSaveInput | GroupSaveInput;

export interface RecordDeleteResult {
  deleted: Uuid[];
  /** Children a group removal moved one level up. */
  moved: Uuid[];
}

export type MovePosition = { kind: 'end' } | { kind: 'before'; record_id: Uuid } | { kind: 'after'; record_id: Uuid };

export interface RecordMoveInput {
  vault_id: Uuid;
  record_ids: Uuid[];
  /** Omit or null for the top level. */
  target_group_id?: Uuid | null;
  /** Default `{kind: 'end'}`. */
  position?: MovePosition;
}

export interface SecretFieldRef {
  vault_id: Uuid;
  record_id: Uuid;
  path: HostSecretPath;
}

export interface SecretValue {
  value: string;
}

export type SecretCopyTarget =
  | { kind: 'record_field'; vault_id: Uuid; record_id: Uuid; path: HostSecretPath }
  | { kind: 'recovery_key' };

// ---------------------------------------------------------------------------
// Settings (docs/data-model.md, section 16.1)
// ---------------------------------------------------------------------------

/** JSON values. */
export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

/** Every known key with its effective value (defaults applied). */
export type SettingsValues = Record<string, JsonValue>;

/** Typed view of the keys used in stage 1. */
export interface Stage1Settings {
  'appearance.ui_theme': 'auto' | 'light' | 'dark';
  'appearance.ui_zoom': number;
  /** `null` = never. */
  'security.auto_lock_minutes': 1 | 5 | 15 | 30 | 60 | null;
  'view.hosts.layout': 'grid' | 'list';
  'view.hosts.sort': SortMode;
  'view.details_panel_width': number;
}

export interface SettingsSetInput {
  values?: Partial<Record<string, JsonValue>>;
  /** Keys reset to their defaults. */
  reset?: string[];
}

// ---------------------------------------------------------------------------
// Notifications (docs/data-model.md, section 16.4)
// ---------------------------------------------------------------------------

export type NotificationLevel = 'info' | 'success' | 'warning' | 'error';
export type NotificationSourceKind = 'sftp' | 'session' | 'forwarding' | 'sync' | 'update' | 'app';

export interface NotificationSource {
  kind: NotificationSourceKind;
  ref?: string;
}

export interface Notification {
  id: number;
  level: NotificationLevel;
  /** i18n key. */
  message_key: string;
  params: Record<string, string>;
  source: NotificationSource;
  created_at: Timestamp;
  read: boolean;
}

export interface NotificationList {
  /** Newest first. */
  items: Notification[];
  unread: number;
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

export interface ClipboardWriteInput {
  text: string;
}

export interface ClipboardText {
  /** `null` when the clipboard holds no text. */
  text: string | null;
}

// ---------------------------------------------------------------------------
// e2e mocks (only in builds with the `e2e-mocks` feature)
// ---------------------------------------------------------------------------

export type E2eMockAction =
  | { kind: 'clipboard_read' }
  | { kind: 'clipboard_write'; text: string }
  | { kind: 'launcher_requests' }
  | { kind: 'secret_store_set_available'; available: boolean }
  | { kind: 'secret_store_accounts' }
  | { kind: 'notify'; level: NotificationLevel; message_key: string; params?: Record<string, string> }
  | { kind: 'clock_advance'; seconds: number }
  | { kind: 'inject_link'; url: string };

/** Result of `e2e_mock`; the member set depends on the action (plan, "e2e_mock"). */
export interface E2eMockResult {
  text?: string | null;
  urls?: string[];
  accounts?: string[];
}

// ---------------------------------------------------------------------------
// Events (docs/architecture.md, section 3.5)
// ---------------------------------------------------------------------------

export interface LockChangedEvent {
  v: number;
  locked: boolean;
  state: LockState;
}

export interface RecordsChangedEvent {
  v: number;
  vault_id: Uuid;
  record_ids: Uuid[];
  /** Record class codes (2 = hosts). */
  classes: number[];
}

export interface VaultsChangedEvent {
  v: number;
  vault_ids: Uuid[];
}

export interface SettingsChangedEvent {
  v: number;
  keys: string[];
}

export interface NotificationsChangedEvent {
  v: number;
  unread: number;
  latest?: Notification;
}

/** Window-targeted: a `shellpp://host/<id>` link, delivered after unlock. */
export interface LinkOpenEvent {
  kind: 'host';
  record_id: Uuid;
}

/** Window-targeted (macOS native menu bar): an app menu item id such as `app-menu-settings`. */
export interface MenuActionEvent {
  id: string;
}

/** Event name to payload. Broadcast events carry `v`. */
export interface EventMap {
  'lock:changed': LockChangedEvent;
  'records:changed': RecordsChangedEvent;
  'vaults:changed': VaultsChangedEvent;
  'settings:changed': SettingsChangedEvent;
  'notifications:changed': NotificationsChangedEvent;
  'link:open': LinkOpenEvent;
  'menu:action': MenuActionEvent;
}

export type EventName = keyof EventMap;

// ---------------------------------------------------------------------------
// Command map: name -> [input, output]. Commands without input use `void`.
// ---------------------------------------------------------------------------

export interface CommandMap {
  // app
  app_info: [void, AppInfo];
  state_snapshot: [void, StateSnapshot];
  window_open: [void, WindowOpenResult];
  activity: [void, null];
  app_exit: [void, null];
  external_open_url: [ExternalOpenInput, null];
  // lock
  lock_status: [void, LockStatus];
  first_run_setup: [FirstRunInput, RecoveryKeyResult];
  unlock: [PasswordInput, LockStatus];
  lock: [void, LockStatus];
  recovery_unlock: [RecoveryUnlockInput, LockStatus];
  password_change: [PasswordChangeInput, null];
  recovery_key_show: [PasswordInput, RecoveryKeyResult];
  recovery_key_reissue: [PasswordInput, RecoveryKeyResult];
  remember_device_set: [RememberDeviceInput, LockStatus];
  // records
  records_query: [RecordsQuery, RecordView[]];
  record_get: [RecordRef, RecordView];
  record_save: [RecordSaveInput, RecordView];
  record_delete: [RecordsRef, RecordDeleteResult];
  record_restore: [RecordsRef, RecordView[]];
  record_move: [RecordMoveInput, RecordView[]];
  record_duplicate: [RecordRef, RecordView];
  secret_reveal: [SecretFieldRef, SecretValue];
  secret_copy: [SecretCopyTarget, null];
  // settings
  settings_get: [void, SettingsValues];
  settings_set: [SettingsSetInput, SettingsValues];
  // notifications
  notifications_list: [void, NotificationList];
  notifications_mark_read: [void, NotificationList];
  // clipboard
  clipboard_write_text: [ClipboardWriteInput, null];
  clipboard_read_text: [void, ClipboardText];
  // e2e only
  e2e_mock: [E2eMockAction, E2eMockResult];
}

export type CommandName = keyof CommandMap;
export type CommandInput<C extends CommandName> = CommandMap[C][0];
export type CommandOutput<C extends CommandName> = CommandMap[C][1];
