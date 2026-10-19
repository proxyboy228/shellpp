// Window workspace of terminal tabs (stage 2 plan, section 6.2): session tabs
// and new-tab pages, their split pane trees, the active tab and the terminal
// sidebar, plus the window's sessions (`SessionInfo` by id, ordered by `rev`).
//
// View state only: ids, layout and the session view models the core sends.
// It survives a lock (the terminals stay mounted under the unlock screen,
// docs/security.md, section 8) and is rebuilt from `StateSnapshot.sessions`
// after a window reload.
//
// The tree and workspace functions below are pure (state in, state out), so
// the reducer rules are unit-tested without React: split limits of
// docs/data-model.md section 12 (2..4 children, at most 16 leaves, depth 4),
// the neighbor rule of UI 4.6 and pane focus by direction (UI 4.7).
//
// Owner: S6. The exported types and functions are the contract used by the
// app shell (lead) and by S7; their bodies may change, their shapes only
// through the plan.

import { useMemo } from 'react';
import { createStore, useStore } from '../../app/store/createStore';
import type { ConnectStep, HostOs, SessionInfo, SessionState, Uuid } from '../../ipc';

/** `t-<n>`, unique per window. */
export type TabId = string;
/** `p-<n>`, unique per window. */
export type PaneId = string;

/** A pane tree (UI 4.7): leaves hold one session or an empty host picker. */
export type PaneNode =
  | { type: 'leaf'; paneId: PaneId; sessionId: Uuid | null }
  | { type: 'split'; direction: 'row' | 'column'; sizes: number[]; children: PaneNode[] };

export type PaneLeaf = Extract<PaneNode, { type: 'leaf' }>;

export interface TerminalTab {
  id: TabId;
  /** `newtab`: the new tab page (UI 6) until a host is chosen. */
  kind: 'session' | 'newtab';
  /** Rename (UI 4.4); null = the session title. */
  customTitle: string | null;
  root: PaneNode | null;
  activePaneId: PaneId | null;
}

/** Terminal sidebar tabs (UI 4.2); `snippets` is stage 5 (hidden). */
export type SidebarTab = 'tools' | 'snippets' | 'history' | 'appearance';

export interface WorkspaceState {
  tabs: readonly TerminalTab[];
  /** The active title bar tab: the persistent Vaults tab or a terminal tab. */
  active: 'vaults' | TabId;
  sidebarOpen: boolean;
  sidebarTab: SidebarTab;
}

export const INITIAL_WORKSPACE: WorkspaceState = {
  tabs: [],
  active: 'vaults',
  sidebarOpen: false,
  sidebarTab: 'tools',
};

export const workspaceStore = createStore<WorkspaceState>(INITIAL_WORKSPACE);

/** Sessions of this window by id (snapshot plus `sessions:changed`, newest `rev` wins). */
export const sessionsStore = createStore<ReadonlyMap<Uuid, SessionInfo>>(new Map());

/** Hooks registered by other modules of the feature to reset their state with the workspace. */
const resetHooks = new Set<() => void>();

/** Registers a function that `resetWorkspace` calls (module state of the feature). */
export function onWorkspaceReset(hook: () => void): void {
  resetHooks.add(hook);
}

/** Resets the workspace and sessions (tests; a window that boots again). */
export function resetWorkspace(): void {
  workspaceStore.set(INITIAL_WORKSPACE);
  sessionsStore.set(new Map());
  for (const hook of resetHooks) {
    hook();
  }
}

export const useWorkspace = () => useStore(workspaceStore);
export const useSessions = () => useStore(sessionsStore);

// ---------------------------------------------------------------------------
// Pane trees
// ---------------------------------------------------------------------------

/** Limits of docs/data-model.md, section 12. */
export const MAX_SPLIT_CHILDREN = 4;
export const MAX_LEAVES = 16;
export const MAX_DEPTH = 4;
/** The smallest share of a split a pane can be dragged to (UI 4.7). */
export const MIN_PANE_SIZE = 0.1;

/** The leaves of a tree, left to right and top to bottom. */
export function leavesOf(node: PaneNode | null): PaneLeaf[] {
  if (node === null) {
    return [];
  }
  if (node.type === 'leaf') {
    return [node];
  }
  return node.children.flatMap(leavesOf);
}

/** Depth of a tree: a leaf is 0, a split of leaves is 1. */
export function depthOf(node: PaneNode | null): number {
  if (node === null || node.type === 'leaf') {
    return 0;
  }
  return 1 + Math.max(...node.children.map(depthOf));
}

export function findLeaf(node: PaneNode | null, paneId: PaneId): PaneLeaf | undefined {
  return leavesOf(node).find((leaf) => leaf.paneId === paneId);
}

function normalizeSizes(sizes: readonly number[]): number[] {
  const total = sizes.reduce((sum, size) => sum + size, 0);
  if (!(total > 0)) {
    return sizes.map(() => 1 / sizes.length);
  }
  return sizes.map((size) => size / total);
}

/**
 * Splits the leaf `paneId`: `row` puts the new pane to the right (split
 * vertically, Ctrl+Shift+D), `column` below (split horizontally,
 * Ctrl+Shift+E). Inside a split of the same direction the new pane becomes a
 * sibling that takes half of the pane's share; otherwise the leaf becomes a
 * split of two. Returns null when a limit would be exceeded or the pane is
 * not in the tree.
 */
export function splitLeaf(
  root: PaneNode,
  paneId: PaneId,
  direction: 'row' | 'column',
  newPaneId: PaneId,
): PaneNode | null {
  if (findLeaf(root, paneId) === undefined || leavesOf(root).length >= MAX_LEAVES) {
    return null;
  }
  const newLeaf: PaneLeaf = { type: 'leaf', paneId: newPaneId, sessionId: null };
  const visit = (node: PaneNode, depth: number): PaneNode | null => {
    if (node.type === 'leaf') {
      if (node.paneId !== paneId) {
        return node;
      }
      if (depth + 1 > MAX_DEPTH) {
        return null;
      }
      return { type: 'split', direction, sizes: [0.5, 0.5], children: [node, newLeaf] };
    }
    const index = node.children.findIndex((child) => child.type === 'leaf' && child.paneId === paneId);
    // A full split of this direction falls through: the leaf itself becomes
    // a nested split of two (one level deeper).
    if (index >= 0 && node.direction === direction && node.children.length < MAX_SPLIT_CHILDREN) {
      const share = node.sizes[index] ?? 1 / node.children.length;
      const children = [...node.children];
      children.splice(index + 1, 0, newLeaf);
      const sizes = [...node.sizes];
      sizes.splice(index, 1, share / 2, share / 2);
      return { ...node, children, sizes: normalizeSizes(sizes) };
    }
    let changed = false;
    const children: PaneNode[] = [];
    for (const child of node.children) {
      const next = visit(child, depth + 1);
      if (next === null) {
        return null;
      }
      changed ||= next !== child;
      children.push(next);
    }
    return changed ? { ...node, children } : node;
  };
  const result = visit(root, 0);
  if (result === null || depthOf(result) > MAX_DEPTH) {
    return null;
  }
  return result;
}

/**
 * Removes the leaf `paneId`. Its share goes to the remaining siblings
 * (proportionally); a split left with one child is replaced by that child.
 * Returns null when the tree becomes empty.
 */
export function removeLeaf(root: PaneNode, paneId: PaneId): PaneNode | null {
  const visit = (node: PaneNode): PaneNode | null => {
    if (node.type === 'leaf') {
      return node.paneId === paneId ? null : node;
    }
    const children: PaneNode[] = [];
    const sizes: number[] = [];
    node.children.forEach((child, index) => {
      const next = visit(child);
      if (next !== null) {
        children.push(next);
        sizes.push(node.sizes[index] ?? 1 / node.children.length);
      }
    });
    if (children.length === 0) {
      return null;
    }
    if (children.length === 1) {
      return children[0] as PaneNode;
    }
    if (children.length === node.children.length && children.every((child, index) => child === node.children[index])) {
      return node;
    }
    return { ...node, children, sizes: normalizeSizes(sizes) };
  };
  return visit(root);
}

/** Sets the session of a leaf (a host chosen in an empty pane, a restored layout). */
export function setLeafSession(root: PaneNode, paneId: PaneId, sessionId: Uuid | null): PaneNode {
  if (root.type === 'leaf') {
    return root.paneId === paneId ? { ...root, sessionId } : root;
  }
  let changed = false;
  const children = root.children.map((child) => {
    const next = setLeafSession(child, paneId, sessionId);
    changed ||= next !== child;
    return next;
  });
  return changed ? { ...root, children } : root;
}

/**
 * Sets the sizes of the split at `path` (child indexes from the root).
 * Sizes are clamped to `MIN_PANE_SIZE` and normalized; a wrong path or count
 * leaves the tree unchanged.
 */
export function setSplitSizes(root: PaneNode, path: readonly number[], sizes: readonly number[]): PaneNode {
  if (root.type === 'leaf') {
    return root;
  }
  if (path.length === 0) {
    if (sizes.length !== root.children.length) {
      return root;
    }
    return { ...root, sizes: normalizeSizes(sizes.map((size) => Math.max(MIN_PANE_SIZE, size))) };
  }
  const [head, ...rest] = path;
  const child = head === undefined ? undefined : root.children[head];
  if (child === undefined || head === undefined) {
    return root;
  }
  const next = setSplitSizes(child, rest, sizes);
  if (next === child) {
    return root;
  }
  const children = [...root.children];
  children[head] = next;
  return { ...root, children };
}

/** A leaf's place in the unit square (x, y, width, height in 0..1). */
export interface PaneRect {
  paneId: PaneId;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The rectangles of every leaf. */
export function paneRects(root: PaneNode | null): PaneRect[] {
  const rects: PaneRect[] = [];
  const visit = (node: PaneNode, x: number, y: number, w: number, h: number) => {
    if (node.type === 'leaf') {
      rects.push({ paneId: node.paneId, x, y, w, h });
      return;
    }
    const sizes = normalizeSizes(node.sizes.length === node.children.length ? node.sizes : node.children.map(() => 1));
    let offset = 0;
    node.children.forEach((child, index) => {
      const share = sizes[index] ?? 0;
      if (node.direction === 'row') {
        visit(child, x + offset * w, y, share * w, h);
      } else {
        visit(child, x, y + offset * h, w, share * h);
      }
      offset += share;
    });
  };
  if (root !== null) {
    visit(root, 0, 0, 1, 1);
  }
  return rects;
}

export type PaneDirection = 'left' | 'right' | 'up' | 'down';

const EPSILON = 1e-6;

/**
 * The pane next to `paneId` in a direction (Ctrl+Alt+arrows, UI 4.7): the
 * closest pane whose edge touches this pane's edge and that overlaps it on
 * the other axis; among several, the one with the largest overlap.
 */
export function neighborPane(root: PaneNode | null, paneId: PaneId, direction: PaneDirection): PaneId | null {
  const rects = paneRects(root);
  const from = rects.find((rect) => rect.paneId === paneId);
  if (from === undefined) {
    return null;
  }
  let best: { paneId: PaneId; distance: number; overlap: number } | null = null;
  for (const rect of rects) {
    if (rect.paneId === paneId) {
      continue;
    }
    let distance: number;
    let overlap: number;
    if (direction === 'left' || direction === 'right') {
      distance = direction === 'right' ? rect.x - (from.x + from.w) : from.x - (rect.x + rect.w);
      overlap = Math.min(from.y + from.h, rect.y + rect.h) - Math.max(from.y, rect.y);
    } else {
      distance = direction === 'down' ? rect.y - (from.y + from.h) : from.y - (rect.y + rect.h);
      overlap = Math.min(from.x + from.w, rect.x + rect.w) - Math.max(from.x, rect.x);
    }
    if (distance < -EPSILON || overlap <= EPSILON) {
      continue;
    }
    if (
      best === null ||
      distance < best.distance - EPSILON ||
      (Math.abs(distance - best.distance) <= EPSILON && overlap > best.overlap)
    ) {
      best = { paneId: rect.paneId, distance, overlap };
    }
  }
  return best?.paneId ?? null;
}

// ---------------------------------------------------------------------------
// Workspace operations
// ---------------------------------------------------------------------------

let tabCounter = 0;
let paneCounter = 0;

onWorkspaceReset(() => {
  tabCounter = 0;
  paneCounter = 0;
});

function highestNumber(ids: readonly string[], prefix: string): number {
  let highest = 0;
  for (const id of ids) {
    if (id.startsWith(prefix)) {
      const value = Number(id.slice(prefix.length));
      if (Number.isInteger(value) && value > highest) {
        highest = value;
      }
    }
  }
  return highest;
}

/** A new tab id, never reused in this window (also after tabs were set from outside). */
export function newTabId(state: WorkspaceState = workspaceStore.get()): TabId {
  tabCounter = Math.max(tabCounter, highestNumber(state.tabs.map((tab) => tab.id), 't-')) + 1;
  return `t-${tabCounter}`;
}

/** A new pane id, never reused in this window. */
export function newPaneId(state: WorkspaceState = workspaceStore.get()): PaneId {
  const ids = state.tabs.flatMap((tab) => leavesOf(tab.root).map((leaf) => leaf.paneId));
  paneCounter = Math.max(paneCounter, highestNumber(ids, 'p-')) + 1;
  return `p-${paneCounter}`;
}

export function findTab(state: WorkspaceState, tabId: TabId): TerminalTab | undefined {
  return state.tabs.find((tab) => tab.id === tabId);
}

/** The tab that shows a pane. */
export function tabOfPane(state: WorkspaceState, paneId: PaneId): TerminalTab | undefined {
  return state.tabs.find((tab) => findLeaf(tab.root, paneId) !== undefined);
}

/** The tab and pane that show a session. */
export function locateSession(
  state: WorkspaceState,
  sessionId: Uuid,
): { tab: TerminalTab; paneId: PaneId } | undefined {
  for (const tab of state.tabs) {
    const leaf = leavesOf(tab.root).find((candidate) => candidate.sessionId === sessionId);
    if (leaf !== undefined) {
      return { tab, paneId: leaf.paneId };
    }
  }
  return undefined;
}

function replaceTab(state: WorkspaceState, tab: TerminalTab): WorkspaceState {
  return { ...state, tabs: state.tabs.map((candidate) => (candidate.id === tab.id ? tab : candidate)) };
}

/** Appends a tab (after `afterTabId` when given) and activates it unless told otherwise. */
export function addTab(
  state: WorkspaceState,
  tab: TerminalTab,
  options: { activate?: boolean; afterTabId?: TabId } = {},
): WorkspaceState {
  const tabs = [...state.tabs];
  const index = options.afterTabId === undefined ? -1 : tabs.findIndex((candidate) => candidate.id === options.afterTabId);
  if (index >= 0) {
    tabs.splice(index + 1, 0, tab);
  } else {
    tabs.push(tab);
  }
  return { ...state, tabs, active: options.activate === false ? state.active : tab.id };
}

/** A one-pane session tab. */
export function sessionTab(tabId: TabId, paneId: PaneId, sessionId: Uuid | null, customTitle: string | null = null): TerminalTab {
  return {
    id: tabId,
    kind: 'session',
    customTitle,
    root: { type: 'leaf', paneId, sessionId },
    activePaneId: paneId,
  };
}

/**
 * Removes a tab. When it was the active tab, the neighbor rule of UI 4.6
 * applies: the tab on the left, else the one on the right, else Vaults.
 */
export function removeTab(state: WorkspaceState, tabId: TabId): WorkspaceState {
  const index = state.tabs.findIndex((tab) => tab.id === tabId);
  if (index < 0) {
    return state;
  }
  const tabs = state.tabs.filter((tab) => tab.id !== tabId);
  let active = state.active;
  if (active === tabId) {
    const neighbor = tabs[index - 1] ?? tabs[index];
    active = neighbor?.id ?? 'vaults';
  }
  return { ...state, tabs, active };
}

/**
 * Removes a pane from its tab; the tab closes when it was its last pane, or
 * when the pane showed the tab's last session and only empty host-picker
 * panes remain (UI 4.6: the tab closes by itself; neighbor rule). The tab's
 * active pane moves to the nearest remaining pane.
 */
export function removePane(state: WorkspaceState, paneId: PaneId): WorkspaceState {
  const tab = tabOfPane(state, paneId);
  if (tab === undefined || tab.root === null) {
    return state;
  }
  const before = leavesOf(tab.root);
  const root = removeLeaf(tab.root, paneId);
  if (root === null) {
    return removeTab(state, tab.id);
  }
  const removed = before.find((leaf) => leaf.paneId === paneId);
  if (removed !== undefined && removed.sessionId !== null && !leavesOf(root).some((leaf) => leaf.sessionId !== null)) {
    return removeTab(state, tab.id);
  }
  let activePaneId = tab.activePaneId;
  if (activePaneId === paneId || activePaneId === null) {
    const index = before.findIndex((leaf) => leaf.paneId === paneId);
    const after = leavesOf(root);
    activePaneId = (after[Math.max(0, index - 1)] ?? after[0])?.paneId ?? null;
  }
  return replaceTab(state, { ...tab, root, activePaneId });
}

/** Removes every pane that shows one of these sessions (closed sessions). */
export function removeSessions(state: WorkspaceState, sessionIds: ReadonlySet<Uuid>): WorkspaceState {
  let next = state;
  for (const tab of state.tabs) {
    for (const leaf of leavesOf(tab.root)) {
      if (leaf.sessionId !== null && sessionIds.has(leaf.sessionId)) {
        next = removePane(next, leaf.paneId);
      }
    }
  }
  return next;
}

/** Splits the active pane of a tab and focuses the new pane (null when a limit is reached). */
export function splitTabPane(
  state: WorkspaceState,
  tabId: TabId,
  direction: 'row' | 'column',
  newPaneId: PaneId,
): WorkspaceState | null {
  const tab = findTab(state, tabId);
  if (tab === undefined || tab.root === null || tab.kind !== 'session') {
    return null;
  }
  const paneId = tab.activePaneId ?? leavesOf(tab.root)[0]?.paneId;
  if (paneId === undefined) {
    return null;
  }
  const root = splitLeaf(tab.root, paneId, direction, newPaneId);
  if (root === null) {
    return null;
  }
  return replaceTab(state, { ...tab, root, activePaneId: newPaneId });
}

/** Makes a pane the active pane of its tab (click, Ctrl+Alt+arrows). */
export function focusPane(state: WorkspaceState, paneId: PaneId): WorkspaceState {
  const tab = tabOfPane(state, paneId);
  if (tab === undefined || tab.activePaneId === paneId) {
    return state;
  }
  return replaceTab(state, { ...tab, activePaneId: paneId });
}

/** Rename (UI 4.4): an empty name restores the session title. The host is not renamed. */
export function renameTab(state: WorkspaceState, tabId: TabId, title: string): WorkspaceState {
  const tab = findTab(state, tabId);
  if (tab === undefined) {
    return state;
  }
  const trimmed = title.trim().slice(0, 256);
  return replaceTab(state, { ...tab, customTitle: trimmed === '' ? null : trimmed });
}

/** Puts a session into a pane; a new-tab page becomes a session tab (UI 6). */
export function fillPane(state: WorkspaceState, tabId: TabId, paneId: PaneId, sessionId: Uuid): WorkspaceState {
  const tab = findTab(state, tabId);
  if (tab === undefined) {
    return state;
  }
  if (tab.kind === 'newtab' || tab.root === null) {
    return replaceTab(state, { ...tab, kind: 'session', root: { type: 'leaf', paneId, sessionId }, activePaneId: paneId });
  }
  if (findLeaf(tab.root, paneId) === undefined) {
    return state;
  }
  return replaceTab(state, { ...tab, root: setLeafSession(tab.root, paneId, sessionId), activePaneId: paneId });
}

/** Sets the sizes of a split of a tab (divider drag). */
export function resizeSplit(state: WorkspaceState, tabId: TabId, path: readonly number[], sizes: readonly number[]): WorkspaceState {
  const tab = findTab(state, tabId);
  if (tab === undefined || tab.root === null) {
    return state;
  }
  const root = setSplitSizes(tab.root, path, sizes);
  return root === tab.root ? state : replaceTab(state, { ...tab, root });
}

// ---------------------------------------------------------------------------
// Tab summaries
// ---------------------------------------------------------------------------

/** One title bar tab or search result (Ctrl+K, UI 6: "open tabs first"). */
export interface TabSummary {
  id: TabId;
  kind: 'session' | 'newtab';
  title: string;
  sessionIds: Uuid[];
  address?: string;
  os?: HostOs;
  state?: SessionState;
  step?: ConnectStep;
}

/** The session shown in the active pane of a tab (or its first session). */
export function activeSessionId(tab: TerminalTab): Uuid | undefined {
  const leaves = leavesOf(tab.root);
  const active = leaves.find((leaf) => leaf.paneId === tab.activePaneId)?.sessionId;
  return active ?? leaves.find((leaf) => leaf.sessionId !== null)?.sessionId ?? undefined;
}

/** The summaries of a workspace's tabs in title bar order. */
export function summarizeTabs(workspace: WorkspaceState, sessions: ReadonlyMap<Uuid, SessionInfo>): TabSummary[] {
  return workspace.tabs.map((tab) => {
    const ids = leavesOf(tab.root)
      .map((leaf) => leaf.sessionId)
      .filter((id): id is Uuid => id !== null);
    const active = activeSessionId(tab);
    const session = active === undefined ? undefined : sessions.get(active);
    return {
      id: tab.id,
      kind: tab.kind,
      title: tab.customTitle ?? session?.title ?? '',
      sessionIds: ids,
      address: session?.address,
      os: session?.os,
      state: session?.state,
      step: session?.step,
    };
  });
}

/** Tab summaries of this window (stable between renders while nothing changes). */
export function useTabSummaries(): TabSummary[] {
  const workspace = useWorkspace();
  const sessions = useSessions();
  return useMemo(() => summarizeTabs(workspace, sessions), [workspace, sessions]);
}
