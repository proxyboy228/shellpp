// Workspace reducer (stage 2 plan, section 6.2): splits and their limits
// (docs/data-model.md 12), pane removal, the neighbor rule of UI 4.6, pane
// focus by direction (UI 4.7), rename, new-tab pages becoming session tabs.

import { afterEach, describe, expect, it } from 'vitest';
import type { SessionInfo } from '../../ipc';
import {
  addTab,
  depthOf,
  fillPane,
  INITIAL_WORKSPACE,
  leavesOf,
  MAX_LEAVES,
  neighborPane,
  newPaneId,
  newTabId,
  removeLeaf,
  removePane,
  removeSessions,
  removeTab,
  renameTab,
  resetWorkspace,
  sessionTab,
  setSplitSizes,
  splitLeaf,
  splitTabPane,
  summarizeTabs,
  type PaneNode,
  type WorkspaceState,
} from './workspace';

afterEach(() => {
  resetWorkspace();
});

const leaf = (paneId: string, sessionId: string | null = null): PaneNode => ({ type: 'leaf', paneId, sessionId });

function withTabs(...ids: string[]): WorkspaceState {
  let state: WorkspaceState = INITIAL_WORKSPACE;
  for (const id of ids) {
    state = addTab(state, sessionTab(id, `p-${id}`, `s-${id}`));
  }
  return state;
}

describe('pane tree', () => {
  it('splits a leaf to the right and below', () => {
    const right = splitLeaf(leaf('p-1'), 'p-1', 'row', 'p-2');
    expect(right).toEqual({ type: 'split', direction: 'row', sizes: [0.5, 0.5], children: [leaf('p-1'), leaf('p-2')] });
    const below = splitLeaf(right as PaneNode, 'p-2', 'column', 'p-3');
    expect(below).toEqual({
      type: 'split',
      direction: 'row',
      sizes: [0.5, 0.5],
      children: [leaf('p-1'), { type: 'split', direction: 'column', sizes: [0.5, 0.5], children: [leaf('p-2'), leaf('p-3')] }],
    });
  });

  it('adds a sibling in a split of the same direction, taking half of the pane share', () => {
    const tree = splitLeaf(leaf('p-1'), 'p-1', 'row', 'p-2') as PaneNode;
    const next = splitLeaf(tree, 'p-2', 'row', 'p-3');
    expect(next).toMatchObject({ direction: 'row', sizes: [0.5, 0.25, 0.25] });
    expect(leavesOf(next).map((item) => item.paneId)).toEqual(['p-1', 'p-2', 'p-3']);
  });

  it('nests when a split already has four children, and respects depth and leaf limits', () => {
    let tree: PaneNode = leaf('p-0');
    for (let index = 1; index < 4; index += 1) {
      tree = splitLeaf(tree, `p-${index - 1}`, 'row', `p-${index}`) as PaneNode;
    }
    expect(tree.type === 'split' && tree.children.length).toBe(4);
    const nested = splitLeaf(tree, 'p-3', 'row', 'p-4') as PaneNode;
    expect(nested.type === 'split' && nested.children.length).toBe(4);
    expect(depthOf(nested)).toBe(2);

    // Depth: alternate directions down one branch.
    let deep: PaneNode = leaf('d-0');
    let last = 'd-0';
    for (let index = 1; index <= 4; index += 1) {
      const next = splitLeaf(deep, last, index % 2 === 0 ? 'row' : 'column', `d-${index}`);
      expect(next).not.toBeNull();
      deep = next as PaneNode;
      last = `d-${index}`;
    }
    expect(depthOf(deep)).toBe(4);
    // A sibling in the deepest split is fine; one more level is not.
    expect(splitLeaf(deep, last, 'row', 'd-5')).not.toBeNull();
    expect(splitLeaf(deep, last, 'column', 'd-5')).toBeNull();

    // Leaves: at most 16 per tab.
    let wide: PaneNode = leaf('w-0');
    let count = 1;
    for (let row = 0; count < MAX_LEAVES; row += 1) {
      const target = leavesOf(wide)[row % leavesOf(wide).length]?.paneId as string;
      const next = splitLeaf(wide, target, row % 2 === 0 ? 'column' : 'row', `w-${count}`);
      if (next !== null) {
        wide = next;
        count += 1;
      }
    }
    expect(leavesOf(wide)).toHaveLength(MAX_LEAVES);
    expect(splitLeaf(wide, 'w-0', 'row', 'w-x')).toBeNull();
    expect(splitLeaf(leaf('p-1'), 'p-9', 'row', 'p-2')).toBeNull();
  });

  it('removes a leaf, gives its share to the siblings and collapses single-child splits', () => {
    const tree: PaneNode = {
      type: 'split',
      direction: 'row',
      sizes: [0.5, 0.25, 0.25],
      children: [leaf('p-1'), leaf('p-2'), leaf('p-3')],
    };
    const without = removeLeaf(tree, 'p-2');
    expect(without).toMatchObject({ sizes: [0.5 / 0.75, 0.25 / 0.75] });
    expect(removeLeaf(removeLeaf(tree, 'p-2') as PaneNode, 'p-3')).toEqual(leaf('p-1'));
    expect(removeLeaf(leaf('p-1'), 'p-1')).toBeNull();
  });

  it('clamps divider sizes to 10% and normalizes them', () => {
    const tree = splitLeaf(leaf('p-1'), 'p-1', 'row', 'p-2') as PaneNode;
    const resized = setSplitSizes(tree, [], [0.01, 0.99]);
    expect(resized.type === 'split' && resized.sizes[0]).toBeCloseTo(0.1 / 1.09, 5);
    expect(setSplitSizes(tree, [], [1])).toBe(tree);
    expect(setSplitSizes(tree, [5], [0.5, 0.5])).toBe(tree);
  });

  it('finds the neighbor pane in each direction', () => {
    // p-1 | (p-2 over p-3)
    const tree: PaneNode = {
      type: 'split',
      direction: 'row',
      sizes: [0.5, 0.5],
      children: [leaf('p-1'), { type: 'split', direction: 'column', sizes: [0.5, 0.5], children: [leaf('p-2'), leaf('p-3')] }],
    };
    expect(neighborPane(tree, 'p-1', 'right')).toBe('p-2');
    expect(neighborPane(tree, 'p-3', 'left')).toBe('p-1');
    expect(neighborPane(tree, 'p-2', 'down')).toBe('p-3');
    expect(neighborPane(tree, 'p-3', 'up')).toBe('p-2');
    expect(neighborPane(tree, 'p-1', 'left')).toBeNull();
    expect(neighborPane(tree, 'p-2', 'right')).toBeNull();
  });
});

describe('workspace operations', () => {
  it('applies the neighbor rule: left, else right, else Vaults', () => {
    let state = withTabs('t-1', 't-2', 't-3');
    state = { ...state, active: 't-2' };
    state = removeTab(state, 't-2');
    expect(state.active).toBe('t-1');
    state = removeTab(state, 't-1');
    expect(state.active).toBe('t-3');
    state = removeTab(state, 't-3');
    expect(state.active).toBe('vaults');
    expect(state.tabs).toEqual([]);
  });

  it('keeps the active tab when another tab closes', () => {
    const state = removeTab({ ...withTabs('t-1', 't-2'), active: 't-2' }, 't-1');
    expect(state.active).toBe('t-2');
  });

  it('removes panes and closes the tab with its last pane', () => {
    let state = withTabs('t-1', 't-2');
    const split = splitTabPane(state, 't-2', 'row', 'p-new') as WorkspaceState;
    expect(split.tabs[1]?.activePaneId).toBe('p-new');
    state = removePane(split, 'p-new');
    expect(state.tabs[1]?.activePaneId).toBe('p-t-2');
    expect(state.tabs[1]?.root).toEqual(leaf('p-t-2', 's-t-2'));
    state = removeSessions(state, new Set(['s-t-2']));
    expect(state.tabs.map((tab) => tab.id)).toEqual(['t-1']);
    expect(state.active).toBe('t-1');
  });

  it('closes the tab when its last session ends and only empty panes remain', () => {
    let state = withTabs('t-1', 't-2');
    state = splitTabPane(state, 't-2', 'row', 'p-empty') as WorkspaceState;
    state = removeSessions(state, new Set(['s-t-2']));
    expect(state.tabs.map((tab) => tab.id)).toEqual(['t-1']);
    expect(state.active).toBe('t-1');
    // Closing an empty pane keeps the panes that show sessions.
    state = splitTabPane(state, 't-1', 'column', 'p-picker') as WorkspaceState;
    state = removePane(state, 'p-picker');
    expect(state.tabs[0]?.root).toEqual(leaf('p-t-1', 's-t-1'));
  });

  it('refuses to split a new-tab page', () => {
    const state = addTab(INITIAL_WORKSPACE, { id: 't-1', kind: 'newtab', customTitle: null, root: null, activePaneId: null });
    expect(splitTabPane(state, 't-1', 'row', 'p-1')).toBeNull();
  });

  it('turns a new-tab page into a session tab and fills empty panes', () => {
    let state = addTab(INITIAL_WORKSPACE, { id: 't-1', kind: 'newtab', customTitle: null, root: null, activePaneId: null });
    state = fillPane(state, 't-1', 'p-1', 's-1');
    expect(state.tabs[0]).toMatchObject({ kind: 'session', activePaneId: 'p-1', root: leaf('p-1', 's-1') });
    state = splitTabPane(state, 't-1', 'column', 'p-2') as WorkspaceState;
    state = fillPane(state, 't-1', 'p-2', 's-2');
    expect(leavesOf(state.tabs[0]?.root ?? null).map((item) => item.sessionId)).toEqual(['s-1', 's-2']);
  });

  it('renames a tab; an empty name restores the session title', () => {
    const sessions = new Map<string, SessionInfo>([
      [
        's-t-1',
        {
          session_id: 's-t-1',
          rev: 1,
          kind: 'local',
          window: 'w-1',
          state: 'connected',
          title: 'PowerShell',
          terminal_theme: { builtin: 'graphite' },
          backspace: 'default',
          record_history: true,
        },
      ],
    ]);
    let state = renameTab(withTabs('t-1'), 't-1', '  build box  ');
    expect(summarizeTabs(state, sessions)[0]?.title).toBe('build box');
    state = renameTab(state, 't-1', '   ');
    expect(summarizeTabs(state, sessions)[0]).toMatchObject({ title: 'PowerShell', state: 'connected', sessionIds: ['s-t-1'] });
  });

  it('never reuses tab and pane ids', () => {
    const first = newTabId(withTabs('t-7'));
    expect(first).toBe('t-8');
    expect(newTabId(INITIAL_WORKSPACE)).toBe('t-9');
    expect(newPaneId(withTabs('t-1'))).not.toBe(newPaneId(withTabs('t-1')));
  });
});
