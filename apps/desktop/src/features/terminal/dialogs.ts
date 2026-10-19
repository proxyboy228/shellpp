// Transient dialog state of the terminal feature: the exit and window close
// confirmations (UI 4.6), the multi-line paste confirmation (UI 4.3), the
// OSC 8 link confirmation (docs/security.md 12) and the tab being renamed
// (UI 4.4).
//
// Owner: S6.

import { createStore } from '../../app/store/createStore';
import { onWorkspaceReset, type TabId } from './workspace';

export interface CloseGuardRequest {
  kind: 'exit' | 'window';
  /** Active sessions reported by the core. */
  count: number;
}

export interface PasteRequest {
  /** Filtered text (docs/security.md 12), line breaks as CR. */
  text: string;
  lines: number;
  confirm(): void;
}

export interface LinkRequest {
  url: string;
}

export const closeGuardStore = createStore<CloseGuardRequest | null>(null);
export const pasteRequestStore = createStore<PasteRequest | null>(null);
export const linkRequestStore = createStore<LinkRequest | null>(null);
/** The tab whose title is being edited inline. */
export const renamingTabStore = createStore<TabId | null>(null);

onWorkspaceReset(() => {
  closeGuardStore.set(null);
  pasteRequestStore.set(null);
  linkRequestStore.set(null);
  renamingTabStore.set(null);
});
