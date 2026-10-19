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
  | 'internal'
  // stage 2
  | 'cancelled'
  | 'ssh_error'
  | 'host_key_revoked';

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
  /**
   * `{errors: FieldError[]}` for `validation_failed`, `{found: number}` for
   * `newer_schema`, `{reason: SshReason}` for `ssh_error`, else `{}`.
   */
  details: { errors?: FieldError[]; found?: number; reason?: SshReason };
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
  /** When the app process started (RFC 3339): `session.last` saved before it is the previous launch's (UI 6 Restore). */
  launched_at?: string;
  /** Sessions owned by this window (stage 2). */
  sessions: SessionInfo[];
  /** Set once, in the first snapshot of a window opened with an intent. */
  intent?: WindowIntent;
}

export interface WindowOpenResult {
  label: string;
}

export interface ExternalOpenInput {
  /** `https:` only; `http:` too when `source` is `terminal` (terminal links, docs/security.md 12, 13.3). */
  url: string;
  source?: 'terminal';
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
  | { kind: 'inject_link'; url: string }
  // stage 2: seeds a known_hosts entry (hashed and revoked scenarios of UI 10.1, 10.2)
  | ({ kind: 'known_host_add' } & KnownHostAddInput);

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
  'sessions:changed': SessionsChangedEvent;
  'window:close-requested': WindowCloseRequestedEvent;
}

export type EventName = keyof EventMap;

// ---------------------------------------------------------------------------
// Stage 2: sessions, local terminal, known hosts, keys, history, recent
// connections, windows (stage 2 plan, section 5). Mirrors the stage 2 section
// of crates/core/src/service/dto.rs.
// ---------------------------------------------------------------------------

/** `details.reason` of `ssh_error` (crates/core/src/error.rs `SshReason`); text `ssh_reason.<reason>`. */
export type SshReason =
  | 'not_found'
  | 'refused'
  | 'timeout'
  | 'network'
  | 'host_key_rejected'
  | 'auth_failed'
  | 'protocol'
  | 'channel_failed'
  | 'disconnected'
  | 'key_unusable';

/** What an SSH session connects to. */
export type SessionTarget =
  | { kind: 'host'; vault_id: Uuid; record_id: Uuid }
  | { kind: 'quick'; address: string; port?: number; username?: string };

export type SessionKind = 'ssh' | 'local';

/** Session lifecycle (UI 4.6, 4.8; plan 5.4). `closed` is final: the tab closes. */
export type SessionState = 'connecting' | 'connected' | 'ended' | 'dropped' | 'failed' | 'closed';

export type ConnectStep = 'resolving' | 'connecting' | 'host_key' | 'authenticating' | 'opening_session';

export type OsFamily = 'linux' | 'windows' | 'macos' | 'bsd' | 'other';

export type Backspace = 'default' | 'del' | 'ctrl_h';

/** Why a session failed or dropped. */
export interface SessionError {
  code: ErrorCode;
  /** For `ssh_error`. */
  reason?: SshReason;
}

/** Autoreconnect state of a dropped session (UI 4.6). */
export interface ReconnectInfo {
  /** 1-based number of the next (or running) attempt. */
  attempt: number;
  /** Milliseconds until the next attempt when the event was emitted; absent while an attempt runs or while locked. */
  next_in_ms?: number;
  waiting_for_unlock: boolean;
  in_progress: boolean;
}

/** A presented host key (UI 10.2). */
export interface HostKeyInfo {
  address: string;
  port: number;
  key_type: string;
  /** `SHA256:...` */
  fingerprint: string;
}

export interface HostKeyUnknownPrompt {
  kind: 'host_key_unknown';
  key: HostKeyInfo;
}

export interface HostKeyChangedPrompt {
  kind: 'host_key_changed';
  key: HostKeyInfo;
  old_key_type: string;
  old_fingerprint: string;
}

export type PromptKind =
  | HostKeyUnknownPrompt
  | HostKeyChangedPrompt
  | { kind: 'username' }
  | { kind: 'password'; username?: string; error: boolean; can_save: boolean }
  | { kind: 'passphrase'; key_label?: string; error: boolean; can_save: boolean }
  | {
      kind: 'keyboard_interactive';
      /** Server texts: untrusted, rendered as plain text in a frame labeled with the host. */
      name: string;
      instruction: string;
      prompt: string;
      /** `false`: mask the answer. */
      echo: boolean;
      index: number;
      count: number;
      error: boolean;
    };

/** A pending question of a session; answered with `session_prompt_reply`. */
export type SessionPrompt = PromptKind & { prompt_id: number };

/** A session as its window sees it: `sessions:changed` payload and `StateSnapshot.sessions`. */
export interface SessionInfo {
  session_id: Uuid;
  /** Apply an event only when its `rev` is greater than the one the window has. */
  rev: number;
  kind: SessionKind;
  /** Owning window label. */
  window: string;
  state: SessionState;
  step?: ConnectStep;
  /** Host label or address; the shell label for a local session. */
  title: string;
  /** SSH: what to connect to again (Duplicate, Retry, Restore). */
  target?: SessionTarget;
  /** Local: the shell id. */
  shell_id?: string;
  address?: string;
  port?: number;
  username?: string;
  os?: HostOs;
  /** Effective theme reference. */
  terminal_theme: ThemeRef;
  backspace: Backspace;
  /** Send history candidates only when true. */
  record_history: boolean;
  prompt?: SessionPrompt;
  error?: SessionError;
  /** The presented key when `error.code` is `host_key_revoked`. */
  host_key?: HostKeyInfo;
  exit_status?: number;
  reconnect?: ReconnectInfo;
}

/**
 * An IPC channel created with `getBackend().createChannel()`, passed inside a
 * command input (Tauri serializes it as `__CHANNEL__:<id>`). Opaque.
 */
export interface IpcChannel<T> {
  readonly __ipcChannel: true;
  /** Type marker only. */
  readonly __message?: T;
}

export interface SessionOpenInput {
  target: SessionTarget;
  cols: number;
  rows: number;
}

export interface LocalOpenInput {
  /** An id from `local_shells`; omit for the `terminal.local_shell` setting or the default. */
  shell_id?: string | null;
  cols: number;
  rows: number;
}

/** `session_attach`: replays buffered output (at most 256 KiB), then streams live output (raw bytes). */
export interface SessionAttachInput {
  session_id: Uuid;
  on_output: IpcChannel<ArrayBuffer>;
}

export interface SessionRef {
  session_id: Uuid;
}

/** A typed command line captured at Enter (docs/security.md 10.1 rules 1-3). */
export interface HistoryCandidate {
  command: string;
  /** Text before the prompt boundary (or the last non-empty line above). Never stored. */
  prompt_text: string;
  typed_printable: boolean;
  alt_screen: boolean;
}

export interface SessionWriteInput {
  session_id: Uuid;
  data: string;
  /** xterm.js `onBinary`: every UTF-16 code unit is one byte. */
  binary?: boolean;
  /** Only on the write that carries the Enter of a typed command line. */
  history?: HistoryCandidate;
}

/** Input the app sends on its own (history insert); never recorded in history. */
export interface SessionInjectInput {
  session_id: Uuid;
  data: string;
}

export interface SessionResizeInput {
  session_id: Uuid;
  cols: number;
  rows: number;
}

/**
 * `session_ack`: the terminal processed `bytes` more bytes of the output it
 * received (flow control: the host stops reading output while more than
 * 2 MiB are unacknowledged). Accepted while locked.
 */
export interface SessionAckInput {
  session_id: Uuid;
  bytes: number;
}

export type HostKeyDecision = 'accept_save' | 'connect_once' | 'replace' | 'cancel';

export type PromptReply =
  | { kind: 'host_key'; decision: HostKeyDecision }
  | { kind: 'text'; value: string; save?: boolean }
  | { kind: 'cancel' };

export interface SessionPromptReplyInput {
  session_id: Uuid;
  prompt_id: number;
  reply: PromptReply;
}

export interface LocalShell {
  id: string;
  label: string;
}

export interface LocalShellsResult {
  shells: LocalShell[];
  default_id?: string;
}

export type KnownHostSource = 'tofu' | 'replaced' | 'import';

/** A known host entry (UI 10.1). */
export interface KnownHostView {
  vault_id: Uuid;
  record_id: Uuid;
  /** "Date added". */
  created_at: Timestamp;
  modified_at: Timestamp;
  /** Stored host: name, address, pattern or `|1|salt|hash`. */
  host: string;
  port?: number;
  hashed: boolean;
  /** `host` for port 22, `[host]:port` otherwise; absent for hashed entries. */
  display_address?: string;
  key_type: string;
  fingerprint: string;
  revoked: boolean;
  source: KnownHostSource;
  comment?: string;
  /** Saved hosts this entry matches ("saved hosts with this address"). */
  host_ids: Uuid[];
}

/** A key record without secrets. */
export interface KeyView {
  vault_id: Uuid;
  record_id: Uuid;
  created_at: Timestamp;
  modified_at: Timestamp;
  label: string;
  /** `ssh-ed25519`, `ecdsa-sha2-nistp256`, ..., or `unknown`. */
  algorithm: string;
  fingerprint?: string;
  encrypted: boolean;
  passphrase_saved: boolean;
}

/** "+ -> Key" (UI 3.1). */
export interface KeyImportInput {
  vault_id: Uuid;
  label?: string;
  /** OpenSSH private key text. */
  private_key: string;
  passphrase?: string;
  save_passphrase?: boolean;
}

export interface KeyImportResult {
  key: KeyView;
  /** False when the same key (fingerprint) already existed. */
  created: boolean;
}

export interface HistoryListInput {
  /** Omit for entries without a host (local terminals, quick connects). */
  host?: RecordRef | null;
  text?: string;
  /** Default 500, at most 5000. */
  limit?: number;
}

export interface HistoryEntryView {
  vault_id: Uuid;
  record_id: Uuid;
  command: string;
  executed_at: Timestamp;
}

export interface HistoryClearInput {
  host?: RecordRef | null;
}

export interface HistoryClearResult {
  removed: number;
}

/** A row of Recent connections (UI 6). */
export interface RecentConnection {
  host?: RecordRef;
  title: string;
  address: string;
  port: number;
  username?: string;
  os?: HostOs;
  /** Vault name, then group names from the top level ("vault / group"). */
  path: string[];
  last_at: Timestamp;
}

/** What a window opened by `window_open` does first (delivered once in its snapshot). */
export type WindowIntent =
  | { kind: 'open_session'; target: SessionTarget }
  | { kind: 'open_local'; shell_id?: string };

export interface WindowOpenInput {
  intent?: WindowIntent;
}

export interface ExitInput {
  /** The user confirmed closing active sessions (UI 4.6). */
  confirmed?: boolean;
}

export interface ExitResult {
  /** Nothing was closed; ask the user and call again with `confirmed: true`. */
  needs_confirmation: boolean;
  active_sessions: number;
}

/** One known_hosts entry for the e2e mock action `known_host_add` (and the stage 3 import). */
export interface KnownHostAddInput {
  host: string;
  port?: number;
  key_type: string;
  public_key: string;
  revoked?: boolean;
  comment?: string;
}

/** Setting `session.last` (docs/data-model.md 16.3): tabs of the last exit, ids and layout only. */
export type LastSessionPane =
  | { session: { kind: 'ssh' | 'local'; host?: RecordRef } }
  | { direction: 'row' | 'column'; sizes: number[]; children: LastSessionPane[] };

export interface LastSession {
  saved_at: Timestamp;
  tabs: { root: LastSessionPane }[];
}

/** Typed view of the settings keys read in stage 2 (no Terminal settings screen until stage 6). */
export interface Stage2Settings {
  'terminal.autoreconnect': boolean;
  'terminal.record_history': boolean;
  'terminal.copy_on_select': boolean;
  'terminal.bell_sound': boolean;
  'terminal.bold_is_bright': boolean;
  'terminal.cursor_style': 'block' | 'bar' | 'underline';
  'terminal.cursor_blink': boolean;
  'terminal.term': 'xterm' | 'xterm-256color' | 'vt100' | 'linux';
  'terminal.font_family': string;
  'terminal.font_size': number;
  'terminal.line_height': number;
  'terminal.confirm_multiline_paste': boolean;
  'terminal.default_theme': ThemeRef;
  'terminal.os_detection_enabled': boolean;
  'terminal.local_shell': string | null;
  'terminal.keepalive_s': number;
  'view.known_hosts.layout': 'grid' | 'list';
  'view.known_hosts.sort': 'name' | 'created' | 'modified';
  'session.last': LastSession | null;
}

/** Window-targeted, ordered by `rev` per session (not by `v`). */
export type SessionsChangedEvent = SessionInfo;

/** Window-targeted: the OS asked to close this window while it has active sessions (UI 4.6). */
export type WindowCloseRequestedEvent = Record<string, never>;

// ---------------------------------------------------------------------------
// Command map: name -> [input, output]. Commands without input use `void`.
// ---------------------------------------------------------------------------

export interface CommandMap {
  // app
  app_info: [void, AppInfo];
  state_snapshot: [void, StateSnapshot];
  window_open: [WindowOpenInput | void, WindowOpenResult];
  activity: [void, null];
  app_exit: [ExitInput | void, ExitResult];
  window_close: [ExitInput | void, ExitResult];
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
  // sessions (stage 2)
  session_open: [SessionOpenInput, SessionInfo];
  session_attach: [SessionAttachInput, SessionInfo];
  session_write: [SessionWriteInput, null];
  session_inject: [SessionInjectInput, null];
  session_resize: [SessionResizeInput, null];
  session_prompt_reply: [SessionPromptReplyInput, null];
  session_cancel: [SessionRef, null];
  session_close: [SessionRef, null];
  session_reconnect: [SessionRef, SessionInfo];
  session_ack: [SessionAckInput, null];
  // local terminal
  local_shells: [void, LocalShellsResult];
  local_open: [LocalOpenInput, SessionInfo];
  // known hosts
  known_hosts_list: [void, KnownHostView[]];
  known_host_remove: [RecordsRef, null];
  // keys ("+ -> Key")
  keys_list: [void, KeyView[]];
  key_import: [KeyImportInput, KeyImportResult];
  // history and recent connections
  history_list: [HistoryListInput, HistoryEntryView[]];
  history_delete: [RecordsRef, null];
  history_clear: [HistoryClearInput, HistoryClearResult];
  recent_connections: [void, RecentConnection[]];
  // e2e only
  e2e_mock: [E2eMockAction, E2eMockResult];
}

export type CommandName = keyof CommandMap;
export type CommandInput<C extends CommandName> = CommandMap[C][0];
export type CommandOutput<C extends CommandName> = CommandMap[C][1];
