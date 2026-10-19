// Last session (stage 2 plan, decision 16; docs/data-model.md 16.3): the
// window's session tabs and their layouts, ids only, written to the setting
// `session.last` 2 s after a change and before exit; Restore on the new tab
// page reopens the tabs of the previous app launch (UI 6). Saved: SSH
// sessions of saved hosts and local terminals; quick connects, empty panes
// and new-tab pages are skipped; titles are not stored.
//
// - Restore reads `session.last` once, at the window's first unlocked
//   snapshot, and only a value saved before this app launch
//   (`state_snapshot.launched_at`). The writer keeps updating the setting
//   for the next launch, which never feeds this launch's Restore; Restore
//   forgets the value once used (a second click does not open the tabs
//   again).
// - Several windows (D23) share the setting: each window replaces only the
//   tabs it wrote itself during this launch and keeps the others' tabs.
// - The writer starts writing only after this window has shown a restorable
//   tab, so opening the app (or a new tab page) never erases what the next
//   launch would restore.
//
// Owner: S6.

import { settingsStore } from '../../app/store';
import { createStore, useStore } from '../../app/store/createStore';
import {
  settingsSet,
  type JsonValue,
  type LastSession,
  type LastSessionPane,
  type SessionInfo,
  type SettingsValues,
  type Uuid,
} from '../../ipc';
import { onWorkspaceReset, sessionsStore, workspaceStore, type PaneNode, type WorkspaceState } from './workspace';

export const LAST_SESSION_KEY = 'session.last';
export const LAST_SESSION_DEBOUNCE_MS = 2000;
/** docs/data-model.md 12: at most 32 tabs. */
const MAX_TABS = 32;

function serializePane(node: PaneNode, sessions: ReadonlyMap<Uuid, SessionInfo>): LastSessionPane | null {
  if (node.type === 'leaf') {
    const info = node.sessionId === null ? undefined : sessions.get(node.sessionId);
    if (info === undefined) {
      return null;
    }
    if (info.kind === 'local') {
      return { session: { kind: 'local' } };
    }
    if (info.target?.kind === 'host') {
      return { session: { kind: 'ssh', host: { vault_id: info.target.vault_id, record_id: info.target.record_id } } };
    }
    return null;
  }
  const children: LastSessionPane[] = [];
  const sizes: number[] = [];
  node.children.forEach((child, index) => {
    const pane = serializePane(child, sessions);
    if (pane !== null) {
      children.push(pane);
      sizes.push(node.sizes[index] ?? 1 / node.children.length);
    }
  });
  if (children.length === 0) {
    return null;
  }
  if (children.length === 1) {
    return children[0] as LastSessionPane;
  }
  const total = sizes.reduce((sum, size) => sum + size, 0);
  return { direction: node.direction, sizes: sizes.map((size) => size / total), children };
}

/** The restorable tabs of a workspace (without `saved_at`). */
export function serializeTabs(workspace: WorkspaceState, sessions: ReadonlyMap<Uuid, SessionInfo>): LastSession['tabs'] {
  const tabs: LastSession['tabs'] = [];
  for (const tab of workspace.tabs) {
    if (tab.kind !== 'session' || tab.root === null) {
      continue;
    }
    const root = serializePane(tab.root, sessions);
    if (root !== null) {
      tabs.push({ root });
    }
  }
  return tabs.slice(0, MAX_TABS);
}

function isPane(value: unknown): value is LastSessionPane {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  if ('session' in record) {
    const session = record.session as Record<string, unknown> | null;
    if (typeof session !== 'object' || session === null) {
      return false;
    }
    if (session.kind === 'local') {
      return true;
    }
    const host = session.host as Record<string, unknown> | undefined;
    return session.kind === 'ssh' && typeof host?.vault_id === 'string' && typeof host.record_id === 'string';
  }
  return (
    (record.direction === 'row' || record.direction === 'column') &&
    Array.isArray(record.sizes) &&
    Array.isArray(record.children) &&
    record.children.length >= 2 &&
    record.children.length <= 4 &&
    record.sizes.length === record.children.length &&
    record.sizes.every((size) => typeof size === 'number' && size > 0) &&
    record.children.every(isPane)
  );
}

/** The stored last session, when it is valid and has tabs. */
export function readLastSession(value: JsonValue | undefined = settingsStore.get()[LAST_SESSION_KEY]): LastSession | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return null;
  }
  const tabs = (value as Record<string, unknown>).tabs;
  if (!Array.isArray(tabs)) {
    return null;
  }
  const valid = tabs.filter(
    (tab): tab is { root: LastSessionPane } => typeof tab === 'object' && tab !== null && isPane((tab as { root?: unknown }).root),
  );
  if (valid.length === 0) {
    return null;
  }
  const savedAt = (value as Record<string, unknown>).saved_at;
  return { saved_at: typeof savedAt === 'string' ? savedAt : '', tabs: valid };
}

/** JSON with object keys sorted (the core may store members in another order). */
function canonical(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map(canonical).join(',')}]`;
  }
  if (typeof value === 'object' && value !== null) {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(record[key])}`)
      .join(',')}}`;
  }
  return JSON.stringify(value);
}

// --- Restore of the previous launch ------------------------------------------

/** When this app launch started (ms since the epoch); NaN when unknown. */
let launchedAt = Number.NaN;
let launchCaptured = false;
/** The previous launch's tabs for this window's Restore; null once used or when there are none. */
const launchSessionStore = createStore<LastSession | null>(null);

/** True when `value` was saved during this app launch. */
function savedThisLaunch(value: LastSession): boolean {
  const saved = Date.parse(value.saved_at);
  return !Number.isNaN(launchedAt) && !Number.isNaN(saved) && saved >= launchedAt;
}

/**
 * Takes the previous launch's `session.last` for Restore, once per window:
 * at the first unlocked snapshot. A value saved during this launch (by this
 * or another window) is not the previous launch's.
 */
export function captureLaunchSession(settings: SettingsValues, launchedAtIso: string | undefined): void {
  if (launchedAtIso !== undefined) {
    launchedAt = Date.parse(launchedAtIso);
  }
  if (launchCaptured) {
    return;
  }
  launchCaptured = true;
  const value = readLastSession(settings[LAST_SESSION_KEY]);
  launchSessionStore.set(value !== null && !savedThisLaunch(value) ? value : null);
}

/** The previous launch's tabs, forgotten afterwards (Restore). */
export function takeLaunchSession(): LastSession | null {
  const value = launchSessionStore.get();
  launchSessionStore.set(null);
  return value;
}

/** True while the previous launch's tabs can be restored in this window. */
export function useLaunchSessionAvailable(): boolean {
  return useStore(launchSessionStore) !== null;
}

// --- Writer -------------------------------------------------------------------

let restorableSeen = false;
let lastWritten: string | null = null;
/** The tabs this window wrote during this launch (canonical JSON), to replace them next time. */
let mine: string[] = [];
let timer: ReturnType<typeof setTimeout> | null = null;

onWorkspaceReset(() => {
  restorableSeen = false;
  lastWritten = null;
  mine = [];
  launchCaptured = false;
  launchedAt = Number.NaN;
  launchSessionStore.set(null);
  if (timer !== null) {
    clearTimeout(timer);
    timer = null;
  }
});

/** This window's tabs plus the tabs other windows wrote during this launch. */
function merged(tabs: LastSession['tabs']): LastSession['tabs'] {
  const stored = readLastSession();
  const others: LastSession['tabs'] = [];
  if (stored !== null && savedThisLaunch(stored)) {
    const own = [...mine];
    for (const tab of stored.tabs) {
      const index = own.indexOf(canonical(tab));
      if (index >= 0) {
        own.splice(index, 1);
      } else {
        others.push(tab);
      }
    }
  }
  return [...tabs, ...others].slice(0, MAX_TABS);
}

async function write(tabs: LastSession['tabs']): Promise<void> {
  const all = merged(tabs);
  const key = canonical(all);
  if (key === lastWritten) {
    return;
  }
  lastWritten = key;
  const value: LastSession = { saved_at: new Date().toISOString(), tabs: all };
  try {
    const values = await settingsSet({ values: { [LAST_SESSION_KEY]: value as unknown as JsonValue } });
    mine = tabs.map(canonical);
    settingsStore.set(values);
  } catch {
    // Not unlocked or not supported: keep the old value; the next change retries.
    lastWritten = null;
  }
}

function current(): LastSession['tabs'] {
  return serializeTabs(workspaceStore.get(), sessionsStore.get());
}

function schedule(): void {
  const tabs = current();
  if (tabs.length > 0) {
    restorableSeen = true;
  }
  if (!restorableSeen || canonical(merged(tabs)) === lastWritten) {
    return;
  }
  if (timer !== null) {
    clearTimeout(timer);
  }
  timer = setTimeout(() => {
    timer = null;
    void write(current());
  }, LAST_SESSION_DEBOUNCE_MS);
}

/** Writes the current tabs now (before exit). */
export async function flushLastSession(): Promise<void> {
  if (timer !== null) {
    clearTimeout(timer);
    timer = null;
  }
  const tabs = current();
  if (tabs.length > 0) {
    restorableSeen = true;
  }
  if (restorableSeen) {
    await write(tabs);
  }
}

/** Watches the workspace and writes `session.last` debounced; returns a function that stops. */
export function startLastSessionWriter(): () => void {
  const stopWorkspace = workspaceStore.subscribe(schedule);
  const stopSessions = sessionsStore.subscribe(schedule);
  return () => {
    stopWorkspace();
    stopSessions();
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
}
