// View state of the Hosts screen that survives a lock (stage plan decision
// 19): only ids and layout. One store per window (module level). Search text,
// tag filters, drafts and revealed secrets live in component state and are
// dropped when the screen unmounts on lock.

import { useSyncExternalStore } from 'react';
import type { Uuid } from '../../ipc';

export interface OpenPanel {
  type: 'host' | 'group';
  /** `null`: a new record that is still a draft. */
  record_id: Uuid | null;
  /** Folder a draft is created in (decision 17). */
  draft_parent_id: Uuid | null;
  /** Changes whenever a different panel opens; keeps a draft's panel mounted when it gets its id. */
  key: number;
}

export interface HostsViewState {
  /** Open group folder; `null` = top level. */
  folderId: Uuid | null;
  /** Selected host ids, in display order. */
  selectedIds: readonly Uuid[];
  /** Start of a Shift+click range. */
  anchorId: Uuid | null;
  panel: OpenPanel | null;
  /** "Show more" of the host panel is expanded. */
  showMore: boolean;
}

const INITIAL: HostsViewState = {
  folderId: null,
  selectedIds: [],
  anchorId: null,
  panel: null,
  showMore: false,
};

let state: HostsViewState = INITIAL;
let panelKey = 0;
const listeners = new Set<() => void>();

export function getHostsViewState(): HostsViewState {
  return state;
}

export function updateHostsViewState(
  update: Partial<HostsViewState> | ((current: HostsViewState) => Partial<HostsViewState>),
): void {
  const patch = typeof update === 'function' ? update(state) : update;
  const next = { ...state, ...patch };
  const changed = (Object.keys(patch) as (keyof HostsViewState)[]).some((key) => next[key] !== state[key]);
  if (!changed) {
    return;
  }
  state = next;
  for (const listener of listeners) {
    listener();
  }
}

export function subscribeHostsViewState(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useHostsViewState(): HostsViewState {
  return useSyncExternalStore(subscribeHostsViewState, getHostsViewState, getHostsViewState);
}

/** A panel descriptor with a fresh key. */
export function newPanel(type: OpenPanel['type'], recordId: Uuid | null, draftParentId: Uuid | null = null): OpenPanel {
  panelKey += 1;
  return { type, record_id: recordId, draft_parent_id: draftParentId, key: panelKey };
}

/** Drops a draft panel: drafts do not survive a lock or a remount. */
export function dropDraftPanel(): void {
  if (state.panel !== null && state.panel.record_id === null) {
    updateHostsViewState({ panel: null });
  }
}

/** Back to the initial state (tests). */
export function resetHostsViewState(): void {
  state = INITIAL;
  for (const listener of listeners) {
    listener();
  }
}
