// Live terminal views by pane: what shortcuts, the terminal sidebar and the
// title bar ask of the terminal in a pane (focus, copy, paste, search,
// history insert). Each mounted `TerminalView` registers one controller.
//
// Owner: S6.

import { createStore } from '../../app/store/createStore';
import type { SearchResults } from './engine/types';
import { activeSessionId, findTab, leavesOf, onWorkspaceReset, workspaceStore, type PaneId } from './workspace';

export interface PaneController {
  readonly sessionId: string;
  focus(): void;
  /** Ctrl+Shift+C: copies the selection (not a sensitive clipboard value, docs/security.md 9.4). */
  copySelection(): Promise<void>;
  /** Ctrl+Shift+V and right click: pastes the clipboard text through the paste filter. */
  pasteFromClipboard(): Promise<void>;
  selectAll(): void;
  /** History insert: injected input, never recorded (docs/security.md 10.1). */
  insert(text: string): void;
  findNext(term: string): void;
  findPrevious(term: string): void;
  clearSearch(): void;
}

const controllers = new Map<PaneId, PaneController>();

/** Search results of each pane (Tools tab "i of n"). */
export const searchResultsStore = createStore<ReadonlyMap<PaneId, SearchResults>>(new Map());

onWorkspaceReset(() => {
  controllers.clear();
  searchResultsStore.set(new Map());
});

export function registerPaneController(paneId: PaneId, controller: PaneController): () => void {
  controllers.set(paneId, controller);
  return () => {
    if (controllers.get(paneId) === controller) {
      controllers.delete(paneId);
    }
  };
}

export function paneController(paneId: PaneId): PaneController | undefined {
  return controllers.get(paneId);
}

export function setSearchResults(paneId: PaneId, results: SearchResults | null): void {
  searchResultsStore.set((current) => {
    const next = new Map(current);
    if (results === null) {
      next.delete(paneId);
    } else {
      next.set(paneId, results);
    }
    return next;
  });
}

/** The active pane of the active terminal tab. */
export function activePaneId(): PaneId | null {
  const state = workspaceStore.get();
  if (state.active === 'vaults') {
    return null;
  }
  const tab = findTab(state, state.active);
  if (tab === undefined || tab.kind !== 'session') {
    return null;
  }
  return tab.activePaneId ?? leavesOf(tab.root)[0]?.paneId ?? null;
}

/** The controller of the active pane, when it shows a terminal. */
export function activePaneController(): PaneController | undefined {
  const paneId = activePaneId();
  return paneId === null ? undefined : controllers.get(paneId);
}

/** The session shown in the active tab's active pane. */
export function activeTabSessionId(): string | undefined {
  const state = workspaceStore.get();
  const tab = state.active === 'vaults' ? undefined : findTab(state, state.active);
  return tab === undefined ? undefined : activeSessionId(tab);
}
