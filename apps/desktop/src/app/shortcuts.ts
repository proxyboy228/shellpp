// App keyboard shortcuts (UI 1.4, 12.5; D17), PC set (Cmd instead of Ctrl on
// macOS):
//
// - Ctrl+1...9: tab N of the title bar (1 = Vaults), Ctrl+,: Settings;
// - Ctrl+Shift+W: close the tab; Alt+Right / Alt+Left: next / previous tab;
// - Ctrl+L and Ctrl+Shift+L: new local terminal;
// - Ctrl+K and Ctrl+Shift+K: search hosts and tabs.
//
// When focus is in a terminal, only Ctrl+1...9, Ctrl+,, Ctrl+Shift shortcuts
// and Alt+Left/Right are captured; Ctrl+L and Ctrl+K go to the shell. No
// shortcut fires while AltGr is active. Terminal shortcuts (Ctrl+Shift+C/V/A/F/
// D/E, Ctrl+Alt+arrows) are in features/terminal. Shortcuts of hidden
// features (Ctrl+2 SFTP, Ctrl+P, Ctrl+Alt+S) are not bound.
//
// Owner: lead.

import { openSearchPalette } from '../features/newtab';
import { activateTabByNumber, terminalHasFocus } from '../features/terminal';
import type { OsKind } from '../ipc';
import { isVisible } from '../stage';
import { runAppMenuAction } from './AppMenu/actions';
import { hasAppModifier, isMac } from './platform';

/** App menu actions bound to Mod+key (Shift not held). */
const BINDINGS: Readonly<Record<string, string>> = {
  ',': 'app-menu-settings',
};

function altGr(event: KeyboardEvent): boolean {
  return typeof event.getModifierState === 'function' && event.getModifierState('AltGraph');
}

/** Alt alone (no Ctrl, Cmd or Shift): tab navigation. Not on macOS, where Option types characters. */
function altOnly(event: KeyboardEvent, os: OsKind | null): boolean {
  return !isMac(os) && event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

/** Installs the shortcuts on `target`; returns a function that removes them. */
export function installShortcuts(os: OsKind | null, target: Window = window): () => void {
  const handler = (event: KeyboardEvent) => {
    if (event.defaultPrevented || altGr(event)) {
      return;
    }
    const inTerminal = terminalHasFocus();
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;

    if (altOnly(event, os) && isVisible('menu-window-tabs')) {
      if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
        event.preventDefault();
        runAppMenuAction(event.key === 'ArrowRight' ? 'app-menu-window-next-tab' : 'app-menu-window-previous-tab');
      }
      return;
    }
    if (!hasAppModifier(event, os)) {
      return;
    }
    if (!event.shiftKey && /^[1-9]$/.test(event.key)) {
      event.preventDefault();
      activateTabByNumber(Number(event.key));
      return;
    }
    if (!event.shiftKey) {
      const action = BINDINGS[event.key];
      if (action !== undefined) {
        event.preventDefault();
        runAppMenuAction(action);
        return;
      }
    }
    // Ctrl+letter without Shift goes to the shell while a terminal has focus (D17).
    if (inTerminal && !event.shiftKey) {
      return;
    }
    if (key === 'w' && event.shiftKey && isVisible('menu-window-tabs')) {
      event.preventDefault();
      runAppMenuAction('app-menu-window-close-tab');
    } else if (key === 'l' && isVisible('menu-new-local-terminal')) {
      event.preventDefault();
      runAppMenuAction('app-menu-new-local-terminal');
    } else if (key === 'k' && isVisible('shortcut-search-hosts-and-tabs')) {
      event.preventDefault();
      openSearchPalette();
    }
  };
  target.addEventListener('keydown', handler);
  return () => target.removeEventListener('keydown', handler);
}
