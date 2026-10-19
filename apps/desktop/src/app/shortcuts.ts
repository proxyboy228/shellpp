// App keyboard shortcuts of stage 1 (UI 1.4, 12.5): Ctrl+1 (Vaults) and
// Ctrl+, (Settings); Cmd on macOS. Shortcuts of hidden features (Ctrl+2,
// Ctrl+L, Ctrl+Shift+L, Ctrl+K, Ctrl+P `shortcut-snippets`, ...) are not bound.
//
// Owner: W4.

import type { OsKind } from '../ipc';
import { runAppMenuAction } from './AppMenu/actions';
import { hasAppModifier } from './platform';

const BINDINGS: Readonly<Record<string, string>> = {
  '1': 'app-menu-vaults',
  ',': 'app-menu-settings',
};

/** Installs the shortcuts on `target`; returns a function that removes them. */
export function installShortcuts(os: OsKind | null, target: Window = window): () => void {
  const handler = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.shiftKey || !hasAppModifier(event, os)) {
      return;
    }
    const action = BINDINGS[event.key];
    if (action === undefined) {
      return;
    }
    event.preventDefault();
    runAppMenuAction(action);
  };
  target.addEventListener('keydown', handler);
  return () => target.removeEventListener('keydown', handler);
}
