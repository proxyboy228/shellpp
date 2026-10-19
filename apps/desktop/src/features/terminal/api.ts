// Workspace actions used by the app shell (lead: title bar, app menu,
// shortcuts) and by S7 (connect actions, new tab page, Ctrl+K search, split
// pane host picker). Stage 2 plan, section 6.2.
//
// Owner: S6. Signatures are the contract.

import { showErrorToast } from '../../app/errors';
import {
  appExit,
  localOpen,
  sessionClose,
  sessionOpen,
  windowClose,
  windowOpen,
  type LastSessionPane,
  type SessionInfo,
  type SessionTarget,
  type SettingsValues,
  type Uuid,
  type WindowIntent,
} from '../../ipc';
import { closeGuardStore, renamingTabStore } from './dialogs';
import { captureLaunchSession, flushLastSession, takeLaunchSession, useLaunchSessionAvailable } from './lastSession';
import { isClosedSession, mergeSession } from './sessionsSync';
import {
  activeSessionId,
  addTab,
  fillPane,
  findLeaf,
  findTab,
  focusPane,
  leavesOf,
  locateSession,
  neighborPane,
  newPaneId,
  newTabId,
  removePane,
  removeTab,
  renameTab,
  sessionTab,
  sessionsStore,
  splitTabPane,
  workspaceStore,
  type PaneDirection,
  type PaneId,
  type PaneNode,
  type SidebarTab,
  type TabId,
} from './workspace';

/** Where a new session opens. Default: a new tab, activated. */
export interface OpenOptions {
  /** Open in a new app window instead (Alt with a Connect option, "Duplicate in a new window"). */
  newWindow?: boolean;
  /** Turn this new-tab page into the session tab (UI 6: "the session opens in the same tab"). */
  inTab?: TabId;
  /** Connect in this empty split pane (UI 4.7). */
  inPane?: { tabId: TabId; paneId: PaneId };
}

/** Where the session opened; null when it opened in a new window. */
export interface OpenResult {
  tabId: TabId;
  paneId: PaneId;
  sessionId: Uuid;
}

/** Size of the last fitted terminal: new sessions open with it (then resize after attach). */
let lastTerminalSize = { cols: 80, rows: 24 };

/** Remembers the size of a fitted terminal for the next `session_open` / `local_open`. */
export function rememberTerminalSize(cols: number, rows: number): void {
  if (cols >= 1 && cols <= 1000 && rows >= 1 && rows <= 1000) {
    lastTerminalSize = { cols, rows };
  }
}

/**
 * Puts an opened session into the workspace (a new tab, a new-tab page or an
 * empty pane). A snapshot that arrived while the open was in flight may have
 * shown the session in a tab of its own already: that tab is replaced, so a
 * session is never shown in two panes. When the requested pane or new-tab
 * page is gone or filled meanwhile (the user closed it, or picked twice),
 * the session is closed instead of appearing somewhere else.
 */
function placeSession(info: SessionInfo, options: OpenOptions, activate = true): OpenResult | null {
  mergeSession(info);
  if (isClosedSession(info.session_id)) {
    return null;
  }
  const targeted = options.inPane !== undefined || options.inTab !== undefined;
  let result: OpenResult | null = null;
  let orphan = false;
  workspaceStore.set((initial) => {
    let state = initial;
    const shown = locateSession(state, info.session_id);
    if (shown !== undefined) {
      if (!targeted) {
        // Already shown (by a snapshot): that is its place.
        result = { tabId: shown.tab.id, paneId: shown.paneId, sessionId: info.session_id };
        return activate ? { ...focusPane(state, shown.paneId), active: shown.tab.id } : state;
      }
      state = removePane(state, shown.paneId);
    }
    if (options.inPane !== undefined) {
      const { tabId, paneId } = options.inPane;
      const tab = findTab(state, tabId);
      const leaf = tab === undefined ? undefined : findLeaf(tab.root, paneId);
      if (tab !== undefined && leaf !== undefined && leaf.sessionId === null) {
        result = { tabId, paneId, sessionId: info.session_id };
        const filled = fillPane(state, tabId, paneId, info.session_id);
        return activate ? { ...filled, active: tabId } : filled;
      }
    }
    if (options.inTab !== undefined) {
      const tab = findTab(state, options.inTab);
      if (tab !== undefined && tab.kind === 'newtab') {
        const paneId = newPaneId(state);
        result = { tabId: tab.id, paneId, sessionId: info.session_id };
        const filled = fillPane(state, tab.id, paneId, info.session_id);
        return activate ? { ...filled, active: tab.id } : filled;
      }
    }
    if (targeted) {
      orphan = true;
      return state;
    }
    const tabId = newTabId(state);
    const paneId = newPaneId(state);
    result = { tabId, paneId, sessionId: info.session_id };
    return addTab(state, sessionTab(tabId, paneId, info.session_id), { activate });
  });
  if (orphan) {
    // The pane or new-tab page the session was opened for is gone.
    sessionClose(info.session_id).catch(() => undefined);
    return null;
  }
  return result;
}

/** Opens an SSH session (`session_open`) and attaches its terminal. */
export async function openSession(target: SessionTarget, options: OpenOptions = {}): Promise<OpenResult | null> {
  try {
    if (options.newWindow === true) {
      await windowOpen({ kind: 'open_session', target });
      return null;
    }
    const info = await sessionOpen({ target, ...lastTerminalSize });
    return placeSession(info, options);
  } catch (reason) {
    showErrorToast(reason);
    return null;
  }
}

/** Opens a local terminal (`local_open`), UI 1.4 and 2.2. */
export async function openLocalTerminal(options: OpenOptions & { shellId?: string | null } = {}): Promise<OpenResult | null> {
  const { shellId, ...placement } = options;
  try {
    if (placement.newWindow === true) {
      await windowOpen(shellId === undefined || shellId === null ? { kind: 'open_local' } : { kind: 'open_local', shell_id: shellId });
      return null;
    }
    const info = await localOpen({ ...(shellId === undefined ? {} : { shell_id: shellId }), ...lastTerminalSize });
    return placeSession(info, placement);
  } catch (reason) {
    showErrorToast(reason);
    return null;
  }
}

/** "+" in the title bar: a new tab with the new tab page (UI 6). Returns its id. */
export function openNewTab(): TabId {
  const id = newTabId();
  workspaceStore.set((current) =>
    addTab(current, { id, kind: 'newtab', customTitle: null, root: null, activePaneId: null }),
  );
  return id;
}

/** Activates the Vaults tab or a terminal tab. */
export function activateTab(id: 'vaults' | TabId): void {
  workspaceStore.set((current) =>
    current.active === id || (id !== 'vaults' && !current.tabs.some((tab) => tab.id === id))
      ? current
      : { ...current, active: id },
  );
}

/** Ctrl+1...9 (UI 12.5): tab N of the title bar from the left; 1 = Vaults (SFTP is hidden until stage 4). */
export function activateTabByNumber(n: number): void {
  if (n === 1) {
    activateTab('vaults');
    return;
  }
  const tab = workspaceStore.get().tabs[n - 2];
  if (tab !== undefined) {
    activateTab(tab.id);
  }
}

function cycle(step: 1 | -1): void {
  const { tabs, active } = workspaceStore.get();
  const order: ('vaults' | TabId)[] = ['vaults', ...tabs.map((tab) => tab.id)];
  const index = order.indexOf(active);
  const next = order[(index + step + order.length) % order.length];
  if (next !== undefined) {
    activateTab(next);
  }
}

/** Alt+Right, Window > Next tab. */
export function activateNextTab(): void {
  cycle(1);
}

/** Alt+Left, Window > Previous tab. */
export function activatePreviousTab(): void {
  cycle(-1);
}

function closeSessions(ids: readonly (Uuid | null)[]): void {
  for (const id of ids) {
    if (id !== null && sessionsStore.get().has(id)) {
      sessionClose(id).catch(() => undefined);
    }
  }
}

/** Closes a tab and its sessions; the neighbor rule picks the next active tab (UI 4.6). */
export function closeTab(tabId: TabId): void {
  const tab = findTab(workspaceStore.get(), tabId);
  if (tab === undefined) {
    return;
  }
  closeSessions(leavesOf(tab.root).map((leaf) => leaf.sessionId));
  if (renamingTabStore.get() === tabId) {
    renamingTabStore.set(null);
  }
  workspaceStore.set((current) => removeTab(current, tabId));
}

/** Ctrl+Shift+W, Window > Close tab: closes the active terminal tab (UI 4.6 neighbor rule). */
export function closeActiveTab(): void {
  const { active } = workspaceStore.get();
  if (active !== 'vaults') {
    closeTab(active);
  }
}

/** Closes one pane and its session; the last pane closes the tab. */
export function closePane(paneId: PaneId): void {
  const state = workspaceStore.get();
  const tab = state.tabs.find((candidate) => findLeaf(candidate.root, paneId) !== undefined);
  const leaf = tab === undefined ? undefined : findLeaf(tab.root, paneId);
  if (leaf === undefined) {
    return;
  }
  closeSessions([leaf.sessionId]);
  workspaceStore.set((current) => removePane(current, paneId));
}

/**
 * Splits the active pane of a tab (default: the active tab): `row` = a new
 * pane to the right (Ctrl+Shift+D, Split vertically), `column` = below
 * (Ctrl+Shift+E, Split horizontally). The new pane shows the host picker.
 */
export function splitPane(direction: 'row' | 'column', tabId?: TabId): PaneId | null {
  const state = workspaceStore.get();
  const id = tabId ?? (state.active === 'vaults' ? undefined : state.active);
  if (id === undefined) {
    return null;
  }
  const paneId = newPaneId(state);
  const next = splitTabPane(state, id, direction, paneId);
  if (next === null) {
    return null;
  }
  workspaceStore.set({ ...next, active: id });
  return paneId;
}

/** Makes a pane active (click on a pane). */
export function activatePane(paneId: PaneId): void {
  workspaceStore.set((current) => focusPane(current, paneId));
}

/** Ctrl+Alt+arrows (UI 4.7): focus the neighbor pane of the active tab in a direction. */
export function focusPaneInDirection(direction: PaneDirection): void {
  const state = workspaceStore.get();
  const tab = state.active === 'vaults' ? undefined : findTab(state, state.active);
  if (tab === undefined || tab.activePaneId === null) {
    return;
  }
  const next = neighborPane(tab.root, tab.activePaneId, direction);
  if (next !== null) {
    activatePane(next);
  }
}

/** Duplicate (UI 4.4): the active pane's session target again, in a new tab or window. */
export async function duplicateTab(tabId: TabId, options: { newWindow?: boolean } = {}): Promise<OpenResult | null> {
  const tab = findTab(workspaceStore.get(), tabId);
  const sessionId = tab === undefined ? undefined : activeSessionId(tab);
  const info = sessionId === undefined ? undefined : sessionsStore.get().get(sessionId);
  if (info === undefined) {
    return null;
  }
  if (info.kind === 'local') {
    return openLocalTerminal({ shellId: info.shell_id ?? null, newWindow: options.newWindow });
  }
  if (info.target === undefined) {
    return null;
  }
  return openSession(info.target, { newWindow: options.newWindow });
}

/** Rename (UI 4.4): sets the tab's custom title; an empty title restores the session title. */
export function setTabTitle(tabId: TabId, title: string): void {
  workspaceStore.set((current) => renameTab(current, tabId, title));
}

/** Starts the inline rename of a session tab (a new-tab page is not renamed, UI 6). */
export function startRenameTab(tabId: TabId): void {
  if (findTab(workspaceStore.get(), tabId)?.kind === 'session') {
    renamingTabStore.set(tabId);
  }
}

/** The title bar button (UI 1.1, 4.2). */
export function toggleTerminalSidebar(): void {
  workspaceStore.set((current) => ({ ...current, sidebarOpen: !current.sidebarOpen }));
}

/** Opens the terminal sidebar on a tab (Ctrl+Shift+F opens Tools). */
export function openTerminalSidebar(tab: SidebarTab): void {
  workspaceStore.set((current) =>
    current.sidebarOpen && current.sidebarTab === tab ? current : { ...current, sidebarOpen: true, sidebarTab: tab },
  );
}

/** App menu Exit (UI 1.4, 4.6): asks for confirmation while sessions are active. */
export async function requestAppExit(): Promise<void> {
  await flushLastSession();
  const result = await appExit();
  if (result.needs_confirmation) {
    closeGuardStore.set({ kind: 'exit', count: result.active_sessions });
  }
}

/** Title bar close button and `window:close-requested` (UI 4.6). */
export async function requestWindowClose(): Promise<void> {
  const result = await windowClose();
  if (result.needs_confirmation) {
    closeGuardStore.set({ kind: 'window', count: result.active_sessions });
  }
}

/** The user confirmed the close guard dialog. */
export async function confirmCloseGuard(): Promise<void> {
  const request = closeGuardStore.get();
  closeGuardStore.set(null);
  if (request === null) {
    return;
  }
  if (request.kind === 'exit') {
    await flushLastSession();
    await appExit(true);
  } else {
    await windowClose(true);
  }
}

/**
 * True when the previous app launch left tabs this window can restore (new
 * tab page Restore, UI 6); false after Restore was used.
 */
export function useLastSessionAvailable(): boolean {
  return useLaunchSessionAvailable();
}

function buildTree(pane: LastSessionPane, slots: { paneId: PaneId; pane: LastSessionPane }[]): PaneNode {
  if ('session' in pane) {
    // Pane ids come from a counter, so they stay unique before the tab is added.
    const paneId = newPaneId();
    slots.push({ paneId, pane });
    return { type: 'leaf', paneId, sessionId: null };
  }
  return {
    type: 'split',
    direction: pane.direction,
    sizes: [...pane.sizes],
    children: pane.children.map((child) => buildTree(child, slots)),
  };
}

/**
 * New tab page Restore (UI 6; setting `session.last`): reopens the tabs of
 * the previous app launch, one session per pane, once. A tab none of whose
 * sessions could be opened (hosts that are gone) is removed again.
 */
export async function restoreLastSession(): Promise<void> {
  const last = takeLaunchSession();
  if (last === null) {
    return;
  }
  let firstTab: TabId | null = null;
  const restored: TabId[] = [];
  const opens: Promise<unknown>[] = [];
  for (const saved of last.tabs) {
    const slots: { paneId: PaneId; pane: LastSessionPane }[] = [];
    const root = buildTree(saved.root, slots);
    const tabId = newTabId();
    firstTab ??= tabId;
    restored.push(tabId);
    workspaceStore.set((current) =>
      addTab(current, { id: tabId, kind: 'session', customTitle: null, root, activePaneId: slots[0]?.paneId ?? null }, { activate: false }),
    );
    for (const slot of slots) {
      if (!('session' in slot.pane)) {
        continue;
      }
      const session = slot.pane.session;
      const inPane = { tabId, paneId: slot.paneId };
      if (session.kind === 'local') {
        opens.push(restoreInto(() => localOpen({ ...lastTerminalSize }), inPane));
      } else if (session.host !== undefined) {
        const host = session.host;
        opens.push(
          restoreInto(
            () => sessionOpen({ target: { kind: 'host', vault_id: host.vault_id, record_id: host.record_id }, ...lastTerminalSize }),
            inPane,
          ),
        );
      }
    }
  }
  if (firstTab !== null) {
    activateTab(firstTab);
  }
  await Promise.all(opens);
  workspaceStore.set((state) => {
    let next = state;
    for (const tabId of restored) {
      const tab = findTab(next, tabId);
      if (tab !== undefined && !leavesOf(tab.root).some((leaf) => leaf.sessionId !== null)) {
        next = removeTab(next, tabId);
      }
    }
    return next;
  });
}

async function restoreInto(open: () => Promise<SessionInfo>, inPane: { tabId: TabId; paneId: PaneId }): Promise<void> {
  try {
    const info = await open();
    placeSession(info, { inPane }, false);
  } catch {
    // The host is gone or cannot be opened: the pane keeps the host picker.
  }
}

type QuickTarget = Extract<SessionTarget, { kind: 'quick' }>;
let quickConnectOpener: ((target: QuickTarget) => Promise<unknown>) | null = null;

/**
 * The quick connect flow of S7 (UI 2.1: the save offer once connected),
 * used for the quick connect intent of a new window.
 */
export function setQuickConnectOpener(opener: ((target: QuickTarget) => Promise<unknown>) | null): void {
  quickConnectOpener = opener;
}

/** Applies the intent of a window opened with `window_open({intent})`. */
export function handleWindowIntent(intent: WindowIntent): void {
  if (intent.kind === 'open_session') {
    const target = intent.target;
    if (target.kind === 'quick' && quickConnectOpener !== null) {
      void quickConnectOpener(target);
    } else {
      void openSession(target);
    }
  } else {
    void openLocalTerminal({ shellId: intent.shell_id ?? null });
  }
}

/** What a snapshot says about the previous launch's tabs (Restore). */
export interface SnapshotLaunch {
  settings: SettingsValues;
  /** `state_snapshot.launched_at`. */
  launchedAt?: string;
  unlocked: boolean;
}

/**
 * Called with every `state_snapshot`: merges the window's sessions, rebuilds
 * tabs after a reload and, at the first unlocked snapshot, takes the
 * previous launch's tabs for Restore.
 */
export function applySnapshotSessions(
  sessions: readonly SessionInfo[],
  intent: WindowIntent | undefined,
  launch?: SnapshotLaunch,
): void {
  if (launch !== undefined && launch.unlocked) {
    captureLaunchSession(launch.settings, launch.launchedAt);
  }
  for (const info of sessions) {
    mergeSession(info);
  }
  const known = sessionsStore.get();
  workspaceStore.set((state) => {
    let next = state;
    // Panes whose session no longer exists close (a session ended while the window reloaded).
    for (const tab of state.tabs) {
      for (const leaf of leavesOf(tab.root)) {
        if (leaf.sessionId !== null && !known.has(leaf.sessionId)) {
          next = removePane(next, leaf.paneId);
        }
      }
    }
    // Sessions not shown yet get one tab each (after a window reload).
    const shown = new Set(next.tabs.flatMap((tab) => leavesOf(tab.root).map((leaf) => leaf.sessionId)));
    for (const info of sessions) {
      if (!shown.has(info.session_id) && known.has(info.session_id)) {
        const tabId = newTabId(next);
        const paneId = newPaneId(next);
        next = addTab(next, sessionTab(tabId, paneId, info.session_id), { activate: next.active === 'vaults' && next.tabs.length === 0 });
      }
    }
    return next;
  });
  if (intent !== undefined) {
    handleWindowIntent(intent);
  }
}
