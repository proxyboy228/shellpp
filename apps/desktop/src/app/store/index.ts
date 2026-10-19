// Window stores (docs/architecture.md, sections 2.5 and 3.5): each app window
// keeps its own view models, built from a `state_snapshot` plus events. They
// never hold key material or secrets.
//
// - `appInfoStore`: name, version, OS of the build.
// - `windowStore`: boot phase, startup error, window label.
// - `lockStore`: the lock status from the last snapshot.
// - `settingsStore`: effective settings (every key with its value).
// - `notificationsStore`: the in-memory notification list.
// - `vaultsStore`: vaults (empty unless unlocked).
// - `viewStore`: window view state that survives a lock: ids and layout only
//   (plan, decision 19).
//
// Owner: W4.

import type {
  AppInfo,
  ErrorCode,
  JsonValue,
  LockStatus,
  NotificationList,
  SettingsValues,
  Uuid,
  VaultView,
} from '../../ipc';
import { createStore, useStore, useStoreSelector } from './createStore';

export { createStore, useStore, useStoreSelector } from './createStore';
export type { Store } from './createStore';

export type BootPhase = 'booting' | 'ready' | 'error';

export interface WindowState {
  phase: BootPhase;
  /** Startup error (`lock_status` / `state_snapshot` rejected), shown by the fatal error screen. */
  errorCode: ErrorCode | null;
  /** `details.found` of a `newer_schema` error. */
  errorFound: number | null;
  /** `w-<n>` from the snapshot. */
  label: string;
}

/** Window view state kept across a lock (ids and layout only). */
export interface ViewState {
  settingsOpen: boolean;
  sidebarVisible: boolean;
  /** The window is between `first_run_setup` and Finish (recovery key step). */
  firstRunInProgress: boolean;
  /** Which unlock screen is shown while locked. */
  unlockMode: 'password' | 'recovery';
  /** Full screen as requested from View > Full screen. */
  fullscreen: boolean;
}

export const INITIAL_WINDOW_STATE: WindowState = { phase: 'booting', errorCode: null, errorFound: null, label: '' };
export const INITIAL_VIEW_STATE: ViewState = {
  settingsOpen: false,
  sidebarVisible: true,
  firstRunInProgress: false,
  unlockMode: 'password',
  fullscreen: false,
};
const EMPTY_NOTIFICATIONS: NotificationList = { items: [], unread: 0 };
const EMPTY_SETTINGS: SettingsValues = {};
const EMPTY_VAULTS: readonly VaultView[] = [];

export const appInfoStore = createStore<AppInfo | null>(null);
export const windowStore = createStore<WindowState>(INITIAL_WINDOW_STATE);
export const lockStore = createStore<LockStatus | null>(null);
export const settingsStore = createStore<SettingsValues>(EMPTY_SETTINGS);
export const notificationsStore = createStore<NotificationList>(EMPTY_NOTIFICATIONS);
export const vaultsStore = createStore<readonly VaultView[]>(EMPTY_VAULTS);
export const viewStore = createStore<ViewState>(INITIAL_VIEW_STATE);

/** Dialogs of the main window that do not survive a lock. */
export interface DialogsState {
  about: boolean;
}
export const INITIAL_DIALOGS_STATE: DialogsState = { about: false };
export const dialogsStore = createStore<DialogsState>(INITIAL_DIALOGS_STATE);

/** The last `shellpp://host/<id>` link for this window (HostsScreen `selectRequest`). */
export const selectRequestStore = createStore<{ record_id: Uuid; nonce: number } | null>(null);

/** Merges a partial view state. */
export function updateView(patch: Partial<ViewState>): void {
  viewStore.set((current) => {
    const changed = (Object.keys(patch) as (keyof ViewState)[]).some((key) => current[key] !== patch[key]);
    return changed ? { ...current, ...patch } : current;
  });
}

/** Resets every window store (tests, and a window that boots again). */
export function resetStores(): void {
  appInfoStore.set(null);
  windowStore.set(INITIAL_WINDOW_STATE);
  lockStore.set(null);
  settingsStore.set(EMPTY_SETTINGS);
  notificationsStore.set(EMPTY_NOTIFICATIONS);
  vaultsStore.set(EMPTY_VAULTS);
  viewStore.set(INITIAL_VIEW_STATE);
  dialogsStore.set(INITIAL_DIALOGS_STATE);
  selectRequestStore.set(null);
}

/** A numeric setting, or `fallback` when it is missing or not a finite number. */
export function numberSetting(settings: SettingsValues, key: string, fallback: number): number {
  const value: JsonValue | undefined = settings[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

/** A string setting limited to `allowed`, or `fallback`. */
export function enumSetting<T extends string>(
  settings: SettingsValues,
  key: string,
  allowed: readonly T[],
  fallback: T,
): T {
  const value = settings[key];
  return typeof value === 'string' && (allowed as readonly string[]).includes(value) ? (value as T) : fallback;
}

export const useAppInfo = () => useStore(appInfoStore);
export const useWindowState = () => useStore(windowStore);
export const useLockStatus = () => useStore(lockStore);
export const useSettings = () => useStore(settingsStore);
export const useNotifications = () => useStore(notificationsStore);
export const useVaults = () => useStore(vaultsStore);
export const useViewState = () => useStore(viewStore);
export const useDialogs = () => useStore(dialogsStore);
export const useSelectRequest = () => useStore(selectRequestStore);

const selectOs = (info: AppInfo | null) => info?.os ?? null;
/** The OS of the build (`null` until `app_info` answered). */
export const useOs = () => useStoreSelector(appInfoStore, selectOs);
