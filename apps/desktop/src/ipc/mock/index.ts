// In-browser mock of the Rust core for Vitest and `pnpm dev` without Tauri.
// Implements every stage-1 command and event with in-memory state and the
// rules of the plan (lock states, records_query, record_save validation,
// record_move, undo, settings, notifications). Deterministic; fixtures use
// documentation-only addresses (example.com, 192.0.2.0/24, 198.51.100.0/24,
// 203.0.113.0/24).
//
// The mock keeps everything in memory, including the master password it
// compares against; it never protects real data and is not part of release
// builds (`initBackend` picks it only in development and tests).
//
// Owner: W4.

import type { Backend, Unlisten, WindowControls } from '../backend';
import { IpcError } from '../errors';
import type {
  AppInfo,
  CommandInput,
  CommandName,
  CommandOutput,
  ErrorCode,
  EventMap,
  EventName,
  FieldError,
  JsonValue,
  LockState,
  LockStatus,
  Notification,
  NotificationLevel,
  OsKind,
  SettingsValues,
  ValidationReason,
  VaultView,
} from '../types';
import { seedDemoRecords } from './fixtures';
import { createIdFactory, createRandom, fakeRecoveryKey, normalizeRecoveryKey } from './ids';
import { RecordStore } from './records';
import { settingsSchema } from './settings';

export interface MockBackendOptions {
  /** Initial lock state. Default `'first_run'`. */
  initialState?: 'first_run' | 'locked' | 'unlocked';
  /** Master password accepted by `unlock` when starting `'locked'`. */
  password?: string;
  /** Seed hosts and groups (`'demo'`) or start empty (`'empty'`, default). */
  fixtures?: 'empty' | 'demo';
  /** Value of `lock_status().remember_device_available`. Default true. */
  rememberDeviceAvailable?: boolean;
  /** "Remember on this device" is already on (not at first run). Default false. */
  rememberDeviceEnabled?: boolean;
  /** `app_info().os`. Default `'windows'`. */
  os?: OsKind;
  /** `app_info().version`. Default `'0.0.0'`. */
  version?: string;
  /** `state_snapshot().window_label`. Default `'w-1'`. */
  windowLabel?: string;
  /** Recovery key of an existing vault (not at first run). Default: a generated one. */
  recoveryKey?: string;
  /** Initial setting values (stored rows). */
  settings?: Readonly<Record<string, JsonValue>>;
  /** Delay of the password commands, like the KDF (development). Default 0. */
  kdfDelayMs?: number;
  /** Run a 1 s timer for auto-lock and clipboard clearing (development). Default false. */
  timers?: boolean;
  /** Every command except `app_info` and `window_open` fails with this code (a startup error). */
  startupError?: ErrorCode;
}

export interface MockWindowState {
  maximized: boolean;
  fullscreen: boolean;
  minimized: boolean;
  closed: boolean;
  dragging: number;
}

export interface MockBackend extends Backend {
  readonly kind: 'mock';
  /** Test helpers. */
  readonly control: {
    /** Emits `lock:changed` as the auto-lock timer would. */
    autoLock(): void;
    /** Adds a notification and emits `notifications:changed`. */
    notify(messageKey: string, params?: Record<string, string>, level?: NotificationLevel): void;
    /** Text the mock clipboard holds. */
    clipboard(): string | null;
    /** Puts text in the mock clipboard (as another app would). */
    setClipboard(text: string | null): void;
    /** URLs passed to `external_open_url`. */
    openedUrls(): readonly string[];
    /** The current lock state. */
    lockState(): LockState;
    /** The recovery key the vault was set up with (null before first run). */
    recoveryKey(): string | null;
    /** Labels returned by `window_open`. */
    windowsOpened(): readonly string[];
    /** True after `app_exit`. */
    exitRequested(): boolean;
    /** Names of the commands invoked so far, in order. */
    calls(): readonly string[];
    /** Number of `activity` calls. */
    activityCount(): number;
    /** Changes whether an OS keychain is available. */
    setRememberDeviceAvailable(available: boolean): void;
    /** Moves the mock clock forward and runs the timers once (auto-lock, clipboard clearing, undo window). */
    advanceClock(milliseconds: number): void;
    /** Sends `link:open` to this window. */
    emitLink(recordId: string): void;
    /** Sends `menu:action` to this window (macOS native menu). */
    emitMenuAction(id: string): void;
    /** Window state changed through the window controls. */
    windowState(): Readonly<MockWindowState>;
  };
}

type Handler<C extends CommandName> = (input: CommandInput<C>) => CommandOutput<C> | Promise<CommandOutput<C>>;
type Handlers = { [C in CommandName]: Handler<C> };

/** Commands accepted in every lock state (plan, "Commands"). */
const ALWAYS: ReadonlySet<CommandName> = new Set<CommandName>([
  'app_info',
  'state_snapshot',
  'window_open',
  'activity',
  'app_exit',
  'lock_status',
  'settings_get',
  'e2e_mock',
]);
const MAX_NOTIFICATIONS = 200;
const RECOVERY_GRANT_MS = 5 * 60_000;
const CLIPBOARD_CLEAR_MS = 30_000;
const MAX_CLIPBOARD_BYTES = 1024 * 1024;
const HOSTS_CLASS = 2;

function passwordLongEnough(password: string): boolean {
  return [...password.normalize('NFC')].length >= 10;
}

function validation(field: string, reason: ValidationReason): IpcError {
  return new IpcError('validation_failed', { errors: [{ field, reason }] });
}

function sameJson(a: JsonValue | undefined, b: JsonValue | undefined): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function createMockBackend(options: MockBackendOptions = {}): MockBackend {
  const random = createRandom();
  let clockOffset = 0;
  const now = () => Date.now() + clockOffset;
  const newId = createIdFactory(random, now);
  const os: OsKind = options.os ?? 'windows';
  const schema = settingsSchema(os);
  const windowLabel = options.windowLabel ?? 'w-1';

  let lockState: LockState = options.initialState ?? 'first_run';
  let password: string | null = lockState === 'first_run' ? null : (options.password ?? null);
  let recoveryKey: string | null =
    lockState === 'first_run' ? null : (options.recoveryKey ?? fakeRecoveryKey(random));
  let rememberAvailable = options.rememberDeviceAvailable ?? true;
  let rememberEnabled = lockState === 'first_run' ? false : (options.rememberDeviceEnabled ?? false);
  let v = 0;
  let lastActivity = now();
  let recoveryGrantUntil = 0;
  let clipboard: string | null = null;
  let sensitiveClipboard: { text: string; clearAt: number } | null = null;
  const openedUrls: string[] = [];
  const windowsOpened: string[] = [];
  const calls: string[] = [];
  let activityCount = 0;
  let exitRequested = false;
  let windowCounter = 1;
  const storedSettings = new Map<string, JsonValue>(Object.entries(options.settings ?? {}));
  let notifications: Notification[] = [];
  let nextNotificationId = 1;

  const { id: vaultId } = newId();
  const vault: VaultView = { vault_id: vaultId, kind: 'personal', status: 'active', name: 'Personal', color: 'graphite' };
  const records = new RecordStore({ vaultId, newId, now });
  if (lockState !== 'first_run' && options.fixtures === 'demo') {
    seedDemoRecords(records, random);
  }

  // --- Events -----------------------------------------------------------

  const listeners = new Map<EventName, Set<(payload: never) => void>>();
  const dispatch = <E extends EventName>(event: E, payload: EventMap[E]) => {
    const handlers = listeners.get(event);
    if (handlers === undefined) {
      return;
    }
    // Events arrive asynchronously, as from the Tauri event loop.
    queueMicrotask(() => {
      for (const handler of [...handlers]) {
        (handler as (value: EventMap[E]) => void)(structuredClone(payload));
      }
    });
  };
  /** Broadcast events carry the state version, increased under the same "lock" as the change. */
  const broadcast = <E extends 'lock:changed' | 'records:changed' | 'vaults:changed' | 'settings:changed' | 'notifications:changed'>(
    event: E,
    payload: Omit<EventMap[E], 'v'>,
  ) => {
    v += 1;
    dispatch(event, { ...payload, v } as EventMap[E]);
  };

  // --- State helpers -------------------------------------------------------

  const lockStatus = (): LockStatus => ({
    state: lockState,
    remember_device_available: rememberAvailable,
    remember_device_enabled: rememberEnabled,
  });

  const effectiveSettings = (): SettingsValues => {
    const values: SettingsValues = {};
    for (const [key, spec] of Object.entries(schema)) {
      values[key] = structuredClone(storedSettings.has(key) ? (storedSettings.get(key) as JsonValue) : spec.default);
    }
    return values;
  };

  const notificationList = () => ({
    items: structuredClone(notifications),
    unread: notifications.filter((item) => !item.read).length,
  });

  const clearSensitiveClipboard = () => {
    if (sensitiveClipboard !== null && clipboard === sensitiveClipboard.text) {
      clipboard = null;
    }
    sensitiveClipboard = null;
  };

  const doLock = () => {
    lockState = 'locked';
    records.clearUndo();
    recoveryGrantUntil = 0;
    clearSensitiveClipboard();
    broadcast('lock:changed', { locked: true, state: 'locked' });
  };

  const unlocked = () => {
    lockState = 'unlocked';
    lastActivity = now();
    broadcast('lock:changed', { locked: false, state: 'unlocked' });
  };

  const tick = () => {
    if (sensitiveClipboard !== null && now() >= sensitiveClipboard.clearAt) {
      clearSensitiveClipboard();
    }
    if (lockState === 'unlocked') {
      const minutes = effectiveSettings()['security.auto_lock_minutes'];
      if (typeof minutes === 'number' && now() - lastActivity >= minutes * 60_000) {
        doLock();
      }
    }
  };

  const recordsChanged = (recordIds: readonly string[]) => {
    if (recordIds.length > 0) {
      broadcast('records:changed', { vault_id: vaultId, record_ids: [...recordIds], classes: [HOSTS_CLASS] });
    }
  };

  const addNotification = (level: NotificationLevel, messageKey: string, params: Record<string, string> = {}) => {
    const item: Notification = {
      id: nextNotificationId++,
      level,
      message_key: messageKey,
      params: { ...params },
      source: { kind: 'app' },
      created_at: new Date(now()).toISOString(),
      read: false,
    };
    notifications = [item, ...notifications].slice(0, MAX_NOTIFICATIONS);
    broadcast('notifications:changed', {
      unread: notifications.filter((entry) => !entry.read).length,
      latest: structuredClone(item),
    });
  };

  const checkPassword = (candidate: string) => {
    if (password === null || candidate !== password) {
      throw new IpcError('invalid_password');
    }
  };

  const kdf = async () => {
    const delay = options.kdfDelayMs ?? 0;
    if (delay > 0) {
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  };

  const copyToClipboard = (text: string, sensitive: boolean) => {
    clipboard = text;
    sensitiveClipboard = sensitive ? { text, clearAt: now() + CLIPBOARD_CLEAR_MS } : null;
  };

  // --- Window -------------------------------------------------------------

  const windowState: MockWindowState = { maximized: false, fullscreen: false, minimized: false, closed: false, dragging: 0 };
  const resizeHandlers = new Set<() => void>();
  const resized = () => {
    queueMicrotask(() => {
      for (const handler of [...resizeHandlers]) {
        handler();
      }
    });
  };
  const windowControls: WindowControls = {
    label: windowLabel,
    minimize: async () => {
      windowState.minimized = true;
    },
    toggleMaximize: async () => {
      windowState.maximized = !windowState.maximized;
      resized();
    },
    isMaximized: async () => windowState.maximized,
    close: async () => {
      windowState.closed = true;
    },
    startDragging: async () => {
      windowState.dragging += 1;
    },
    setFullscreen: async (fullscreen) => {
      windowState.fullscreen = fullscreen;
      resized();
    },
    onResized: async (handler) => {
      resizeHandlers.add(handler);
      return () => {
        resizeHandlers.delete(handler);
      };
    },
  };

  // --- Commands -----------------------------------------------------------

  const appInfo: AppInfo = {
    name: 'Shell++',
    version: options.version ?? '0.0.0',
    os,
    arch: 'x86_64',
    debug: true,
    e2e: false,
  };

  const handlers: Handlers = {
    app_info: () => structuredClone(appInfo),
    state_snapshot: () => ({
      v,
      lock: lockStatus(),
      settings: effectiveSettings(),
      notifications: notificationList(),
      vaults: lockState === 'unlocked' ? [structuredClone(vault)] : [],
      window_label: windowLabel,
    }),
    window_open: () => {
      windowCounter += 1;
      const label = `w-${windowCounter}`;
      windowsOpened.push(label);
      return { label };
    },
    activity: () => {
      activityCount += 1;
      lastActivity = now();
      return null;
    },
    app_exit: () => {
      exitRequested = true;
      return null;
    },
    external_open_url: ({ url }) => {
      const parsed = URL.canParse(url) ? new URL(url) : null;
      if (parsed === null || parsed.protocol !== 'https:' || parsed.username !== '' || parsed.password !== '' || url.length > 2048) {
        throw validation('url', 'not_allowed');
      }
      openedUrls.push(url);
      return null;
    },

    lock_status: () => lockStatus(),
    first_run_setup: async ({ password: newPassword }) => {
      if (!passwordLongEnough(newPassword)) {
        throw validation('password', 'too_short');
      }
      await kdf();
      password = newPassword;
      recoveryKey = fakeRecoveryKey(random);
      recoveryGrantUntil = now() + RECOVERY_GRANT_MS;
      if (options.fixtures === 'demo') {
        seedDemoRecords(records, random);
      }
      unlocked();
      broadcast('vaults:changed', { vault_ids: [vaultId] });
      return { recovery_key: recoveryKey };
    },
    unlock: async ({ password: candidate }) => {
      await kdf();
      checkPassword(candidate);
      unlocked();
      return lockStatus();
    },
    lock: () => {
      if (lockState === 'unlocked') {
        doLock();
      }
      return lockStatus();
    },
    recovery_unlock: async ({ recovery_key, new_password }) => {
      const normalized = normalizeRecoveryKey(recovery_key);
      const errors: FieldError[] = [];
      if (normalized === null) {
        errors.push({ field: 'recovery_key', reason: 'invalid' });
      }
      if (!passwordLongEnough(new_password)) {
        errors.push({ field: 'new_password', reason: 'too_short' });
      }
      if (errors.length > 0) {
        throw new IpcError('validation_failed', { errors });
      }
      await kdf();
      if (recoveryKey === null || normalized !== normalizeRecoveryKey(recoveryKey)) {
        throw new IpcError('invalid_recovery_key');
      }
      password = new_password;
      broadcast('vaults:changed', { vault_ids: [vaultId] });
      unlocked();
      return lockStatus();
    },
    password_change: async ({ current_password, new_password }) => {
      await kdf();
      checkPassword(current_password);
      if (!passwordLongEnough(new_password)) {
        throw validation('new_password', 'too_short');
      }
      password = new_password;
      return null;
    },
    recovery_key_show: async ({ password: candidate }) => {
      await kdf();
      checkPassword(candidate);
      recoveryGrantUntil = now() + RECOVERY_GRANT_MS;
      return { recovery_key: recoveryKey ?? '' };
    },
    recovery_key_reissue: async ({ password: candidate }) => {
      await kdf();
      checkPassword(candidate);
      recoveryKey = fakeRecoveryKey(random);
      recoveryGrantUntil = now() + RECOVERY_GRANT_MS;
      broadcast('vaults:changed', { vault_ids: [vaultId] });
      recordsChanged(records.all().map((record) => record.record_id));
      return { recovery_key: recoveryKey };
    },
    remember_device_set: ({ enabled }) => {
      if (enabled && !rememberAvailable) {
        throw new IpcError('unavailable');
      }
      if (rememberEnabled !== enabled) {
        rememberEnabled = enabled;
        // As the core: every window takes a new snapshot for its toggle.
        broadcast('vaults:changed', { vault_ids: [vaultId] });
      }
      return lockStatus();
    },

    records_query: (query) => records.query(query),
    record_get: ({ vault_id, record_id }) => {
      if (vault_id !== vaultId) {
        throw new IpcError('not_found');
      }
      return records.getView(record_id);
    },
    record_save: (input) => {
      const view = records.save(input);
      recordsChanged([view.record_id]);
      return view;
    },
    record_delete: ({ vault_id, record_ids }) => {
      if (vault_id !== vaultId) {
        throw new IpcError('not_found');
      }
      const result = records.remove(record_ids);
      recordsChanged([...result.deleted, ...result.moved]);
      return result;
    },
    record_restore: ({ vault_id, record_ids }) => {
      if (vault_id !== vaultId) {
        throw new IpcError('not_found');
      }
      const views = records.restore(record_ids);
      recordsChanged(views.map((view) => view.record_id));
      return views;
    },
    record_move: (input) => {
      const views = records.move(input);
      recordsChanged(views.map((view) => view.record_id));
      return views;
    },
    record_duplicate: ({ vault_id, record_id }) => {
      if (vault_id !== vaultId) {
        throw new IpcError('not_found');
      }
      const view = records.duplicate(record_id);
      recordsChanged([view.record_id]);
      return view;
    },
    secret_reveal: ({ vault_id, record_id, path }) => {
      if (vault_id !== vaultId) {
        throw new IpcError('not_found');
      }
      return { value: records.secret(record_id, path) };
    },
    secret_copy: (target) => {
      if (target.kind === 'recovery_key') {
        if (recoveryKey === null || now() >= recoveryGrantUntil) {
          throw new IpcError('wrong_state');
        }
        copyToClipboard(recoveryKey, true);
        return null;
      }
      if (target.vault_id !== vaultId) {
        throw new IpcError('not_found');
      }
      copyToClipboard(records.secret(target.record_id, target.path), true);
      return null;
    },

    settings_get: () => effectiveSettings(),
    settings_set: ({ values = {}, reset = [] }) => {
      const errors: FieldError[] = [];
      for (const [key, value] of Object.entries(values)) {
        const spec = schema[key];
        if (spec === undefined || value === undefined) {
          errors.push({ field: key, reason: 'invalid' });
          continue;
        }
        const reason = spec.validate(value);
        if (reason !== null) {
          errors.push({ field: key, reason });
        }
      }
      for (const key of reset) {
        if (schema[key] === undefined) {
          errors.push({ field: key, reason: 'invalid' });
        }
      }
      if (errors.length > 0) {
        throw new IpcError('validation_failed', { errors });
      }
      const before = effectiveSettings();
      for (const key of reset) {
        storedSettings.delete(key);
      }
      for (const [key, value] of Object.entries(values)) {
        storedSettings.set(key, structuredClone(value as JsonValue));
      }
      const after = effectiveSettings();
      const changed = Object.keys(after).filter((key) => !sameJson(before[key], after[key]));
      if (changed.length > 0) {
        broadcast('settings:changed', { keys: changed });
      }
      return after;
    },

    notifications_list: () => notificationList(),
    notifications_mark_read: () => {
      const hadUnread = notifications.some((item) => !item.read);
      notifications = notifications.map((item) => (item.read ? item : { ...item, read: true }));
      if (hadUnread) {
        broadcast('notifications:changed', { unread: 0 });
      }
      return notificationList();
    },

    clipboard_write_text: ({ text }) => {
      if (new TextEncoder().encode(text).length > MAX_CLIPBOARD_BYTES) {
        throw validation('text', 'too_long');
      }
      copyToClipboard(text, false);
      return null;
    },
    clipboard_read_text: () => ({ text: clipboard }),

    e2e_mock: (action) => {
      switch (action.kind) {
        case 'clipboard_read':
          return { text: clipboard };
        case 'clipboard_write':
          copyToClipboard(action.text, false);
          return {};
        case 'launcher_requests':
          return { urls: [...openedUrls] };
        case 'secret_store_set_available':
          rememberAvailable = action.available;
          return {};
        case 'secret_store_accounts':
          return { accounts: rememberEnabled ? ['device-key:mock'] : [] };
        case 'notify':
          addNotification(action.level, action.message_key, action.params);
          return {};
        case 'clock_advance':
          if (action.seconds < 0 || action.seconds > 86_400) {
            throw validation('seconds', 'out_of_range');
          }
          clockOffset += action.seconds * 1000;
          tick();
          return {};
        case 'inject_link': {
          const match = /^shellpp:\/\/host\/([0-9a-f-]{36})$/.exec(action.url);
          if (match !== null && lockState === 'unlocked') {
            dispatch('link:open', { kind: 'host', record_id: match[1] });
          }
          return {};
        }
        default:
          throw validation('kind', 'invalid');
      }
    },
  };

  const guard = (command: CommandName) => {
    if (options.startupError !== undefined && command !== 'app_info' && command !== 'window_open') {
      throw new IpcError(options.startupError);
    }
    if (ALWAYS.has(command)) {
      return;
    }
    if (command === 'first_run_setup') {
      if (lockState !== 'first_run') {
        throw new IpcError('wrong_state');
      }
      return;
    }
    if (command === 'unlock' || command === 'recovery_unlock') {
      if (lockState !== 'locked') {
        throw new IpcError('wrong_state');
      }
      return;
    }
    if (command === 'lock') {
      if (lockState === 'first_run') {
        throw new IpcError('wrong_state');
      }
      return;
    }
    if (lockState !== 'unlocked') {
      throw new IpcError('locked');
    }
  };

  if (options.timers) {
    // Development only: the page lives as long as the mock.
    setInterval(tick, 1000);
  }

  return {
    kind: 'mock',
    async invoke<C extends CommandName>(command: C, input: CommandInput<C>): Promise<CommandOutput<C>> {
      calls.push(command);
      guard(command);
      const handler = handlers[command] as Handler<C>;
      const result = await handler(input === undefined ? input : structuredClone(input));
      return result === null || result === undefined ? result : structuredClone(result);
    },
    listen<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void): Promise<Unlisten> {
      let set = listeners.get(event);
      if (set === undefined) {
        set = new Set();
        listeners.set(event, set);
      }
      const entry = handler as (payload: never) => void;
      set.add(entry);
      return Promise.resolve(() => {
        listeners.get(event)?.delete(entry);
      });
    },
    window: windowControls,
    control: {
      autoLock: () => {
        if (lockState === 'unlocked') {
          doLock();
        }
      },
      notify: (messageKey, params, level = 'info') => addNotification(level, messageKey, params),
      clipboard: () => clipboard,
      setClipboard: (text) => {
        clipboard = text;
        sensitiveClipboard = null;
      },
      openedUrls: () => [...openedUrls],
      lockState: () => lockState,
      recoveryKey: () => recoveryKey,
      windowsOpened: () => [...windowsOpened],
      exitRequested: () => exitRequested,
      calls: () => [...calls],
      activityCount: () => activityCount,
      setRememberDeviceAvailable: (available) => {
        rememberAvailable = available;
      },
      advanceClock: (milliseconds) => {
        clockOffset += milliseconds;
        tick();
      },
      emitLink: (recordId) => dispatch('link:open', { kind: 'host', record_id: recordId }),
      emitMenuAction: (id) => dispatch('menu:action', { id }),
      windowState: () => ({ ...windowState }),
    },
  };
}
