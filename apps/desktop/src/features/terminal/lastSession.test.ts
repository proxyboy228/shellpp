// Last session serialization and writer (stage 2 plan, decision 16;
// docs/data-model.md 16.3).

import { afterEach, describe, expect, it, vi } from 'vitest';
import { settingsStore } from '../../app/store';
import { setBackend, type SessionInfo } from '../../ipc';
import { createMockBackend } from '../../ipc/mock';
import {
  captureLaunchSession,
  flushLastSession,
  readLastSession,
  serializeTabs,
  startLastSessionWriter,
  takeLaunchSession,
  LAST_SESSION_DEBOUNCE_MS,
} from './lastSession';
import {
  addTab,
  INITIAL_WORKSPACE,
  resetWorkspace,
  sessionsStore,
  sessionTab,
  splitTabPane,
  workspaceStore,
  type WorkspaceState,
} from './workspace';

afterEach(() => {
  resetWorkspace();
  setBackend(null);
  settingsStore.set({});
  vi.useRealTimers();
});

const VAULT = '0199f6a1-0000-7000-8000-0000000000a1';
const HOST = '0199f6a1-0000-7000-8000-0000000000b1';

function session(id: string, patch: Partial<SessionInfo>): SessionInfo {
  return {
    session_id: id,
    rev: 1,
    kind: 'ssh',
    window: 'w-1',
    state: 'connected',
    title: id,
    terminal_theme: { builtin: 'graphite' },
    backspace: 'default',
    record_history: true,
    ...patch,
  };
}

const SESSIONS = new Map<string, SessionInfo>([
  ['ssh', session('ssh', { target: { kind: 'host', vault_id: VAULT, record_id: HOST } })],
  ['local', session('local', { kind: 'local', shell_id: 'powershell' })],
  ['quick', session('quick', { target: { kind: 'quick', address: '192.0.2.10' } })],
]);

describe('serializeTabs', () => {
  it('keeps saved hosts and local terminals; skips quick connects, empty panes and new-tab pages', () => {
    let state: WorkspaceState = INITIAL_WORKSPACE;
    state = addTab(state, sessionTab('t-1', 'p-1', 'ssh'));
    state = addTab(state, sessionTab('t-2', 'p-2', 'quick'));
    state = addTab(state, { id: 't-3', kind: 'newtab', customTitle: 'x', root: null, activePaneId: null });
    state = addTab(state, sessionTab('t-4', 'p-4', 'local', 'renamed'));
    state = splitTabPane(state, 't-4', 'row', 'p-5') as WorkspaceState;
    expect(serializeTabs(state, SESSIONS)).toEqual([
      { root: { session: { kind: 'ssh', host: { vault_id: VAULT, record_id: HOST } } } },
      { root: { session: { kind: 'local' } } },
    ]);
  });

  it('keeps split layouts and drops the panes that cannot be restored', () => {
    const state = addTab(INITIAL_WORKSPACE, {
      id: 't-1',
      kind: 'session',
      customTitle: null,
      activePaneId: 'p-1',
      root: {
        type: 'split',
        direction: 'row',
        sizes: [0.5, 0.25, 0.25],
        children: [
          { type: 'leaf', paneId: 'p-1', sessionId: 'ssh' },
          { type: 'leaf', paneId: 'p-2', sessionId: 'quick' },
          { type: 'leaf', paneId: 'p-3', sessionId: 'local' },
        ],
      },
    });
    const [tab] = serializeTabs(state, SESSIONS);
    expect(tab?.root).toEqual({
      direction: 'row',
      sizes: [0.5 / 0.75, 0.25 / 0.75],
      children: [{ session: { kind: 'ssh', host: { vault_id: VAULT, record_id: HOST } } }, { session: { kind: 'local' } }],
    });
  });
});

describe('readLastSession', () => {
  it('accepts valid values with tabs only', () => {
    expect(readLastSession(null)).toBeNull();
    expect(readLastSession({ saved_at: 'x', tabs: [] })).toBeNull();
    expect(readLastSession({ saved_at: 'x', tabs: [{ root: { session: { kind: 'telnet' } } }] })).toBeNull();
    expect(
      readLastSession({
        saved_at: '2026-10-19T10:00:00.000Z',
        tabs: [{ root: { direction: 'row', sizes: [0.5, 0.5], children: [{ session: { kind: 'local' } }, { session: { kind: 'local' } }] } }],
      }),
    ).toMatchObject({ tabs: [{ root: { direction: 'row' } }] });
  });
});

describe('last session writer', () => {
  it('writes 2 s after a change, only once a restorable tab was shown', async () => {
    vi.useFakeTimers();
    const backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
    setBackend(backend);
    const stop = startLastSessionWriter();
    // A new-tab page alone writes nothing (Restore stays available).
    workspaceStore.set(addTab(INITIAL_WORKSPACE, { id: 't-1', kind: 'newtab', customTitle: null, root: null, activePaneId: null }));
    await vi.advanceTimersByTimeAsync(LAST_SESSION_DEBOUNCE_MS + 10);
    expect(backend.control.calls()).not.toContain('settings_set');

    sessionsStore.set(SESSIONS);
    workspaceStore.set((current) => addTab(current, sessionTab('t-2', 'p-2', 'local')));
    await vi.advanceTimersByTimeAsync(LAST_SESSION_DEBOUNCE_MS - 100);
    expect(backend.control.calls()).not.toContain('settings_set');
    await vi.advanceTimersByTimeAsync(200);
    expect(backend.control.calls().filter((name) => name === 'settings_set')).toHaveLength(1);
    expect(readLastSession(settingsStore.get()['session.last'])?.tabs).toEqual([{ root: { session: { kind: 'local' } } }]);

    // Closing every tab afterwards writes an empty list (nothing to restore).
    workspaceStore.set(INITIAL_WORKSPACE);
    await flushLastSession();
    expect(readLastSession(settingsStore.get()['session.last'])).toBeNull();
    stop();
  });
});

describe('restore of the previous launch', () => {
  // This launch started a minute ago; the previous one saved an hour before.
  const LAUNCH = new Date(Date.now() - 60_000).toISOString();
  const previous = { saved_at: new Date(Date.now() - 3_600_000).toISOString(), tabs: [{ root: { session: { kind: 'local' } } }] };

  it('takes the value saved before this launch once, and only at the first capture', () => {
    captureLaunchSession({ 'session.last': previous }, LAUNCH);
    // A later snapshot (after this launch wrote) does not replace it.
    captureLaunchSession({ 'session.last': { saved_at: new Date().toISOString(), tabs: [] } }, LAUNCH);
    expect(takeLaunchSession()?.tabs).toEqual(previous.tabs);
    // Used once.
    expect(takeLaunchSession()).toBeNull();
  });

  it('ignores a value written during this launch (another window, a reload)', () => {
    captureLaunchSession({ 'session.last': { ...previous, saved_at: new Date(Date.now() - 30_000).toISOString() } }, LAUNCH);
    expect(takeLaunchSession()).toBeNull();
  });

  it('keeps the tabs other windows wrote during this launch and replaces its own', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
    setBackend(backend);
    captureLaunchSession({ 'session.last': previous }, LAUNCH);
    const stop = startLastSessionWriter();
    sessionsStore.set(SESSIONS);
    workspaceStore.set(addTab(INITIAL_WORKSPACE, sessionTab('t-1', 'p-1', 'ssh')));
    await vi.advanceTimersByTimeAsync(LAST_SESSION_DEBOUNCE_MS + 10);
    // The previous launch's value is replaced, not merged.
    expect(readLastSession(settingsStore.get()['session.last'])?.tabs).toEqual([
      { root: { session: { kind: 'ssh', host: { vault_id: VAULT, record_id: HOST } } } },
    ]);
    // Another window adds its tab (key order as the core may store it).
    const stored = readLastSession(settingsStore.get()['session.last']);
    settingsStore.set({
      ...settingsStore.get(),
      'session.last': {
        saved_at: new Date().toISOString(),
        tabs: [
          { root: { session: { host: { record_id: HOST, vault_id: VAULT }, kind: 'ssh' } } },
          { root: { session: { kind: 'local' } } },
        ],
      },
    });
    expect(stored).not.toBeNull();
    // This window changes its tabs: its own entry is replaced, the other window's stays.
    workspaceStore.set((current) => addTab(current, sessionTab('t-2', 'p-2', 'local')));
    await flushLastSession();
    expect(readLastSession(settingsStore.get()['session.last'])?.tabs).toEqual([
      { root: { session: { kind: 'ssh', host: { vault_id: VAULT, record_id: HOST } } } },
      { root: { session: { kind: 'local' } } },
      { root: { session: { kind: 'local' } } },
    ]);
    // Writing never touched what Restore offers in this launch.
    expect(takeLaunchSession()?.tabs).toEqual(previous.tabs);
    stop();
  });
});
