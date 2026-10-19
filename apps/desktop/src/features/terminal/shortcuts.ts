// Terminal shortcuts (UI 12.5, D17): Ctrl+Shift+C/V/A (copy, paste, select
// all), Ctrl+Shift+F (search in the terminal), Ctrl+Shift+D/E (splits),
// Ctrl+Alt+arrows (pane focus). App-level shortcuts (Ctrl+1...9, Ctrl+,,
// Ctrl+Shift+W, Alt+arrows, Ctrl+L, Ctrl+K and their Shift variants) are in
// src/app/shortcuts.ts. No shortcut fires while AltGr is active.
//
// While a terminal has focus the app takes only Ctrl+1...9, Ctrl+,, every
// Ctrl+Shift combination, Ctrl+Alt+arrows and Alt+Left/Right; every other
// key (Ctrl+L, Ctrl+K, Ctrl+C ...) goes to the shell. `isAppShortcut` is the
// terminal's key filter: xterm.js does not process the keys it accepts, so
// the shell never gets them.
//
// macOS (D17, Mac set): Cmd replaces Ctrl (Cmd+Shift+C, Cmd+Shift+D, ...);
// pane focus is Cmd+Option+arrows; Ctrl+letter always goes to the shell and
// Option alone types characters. Every Cmd combination belongs to the app or
// the system, never to the shell.
//
// Owner: S6.

import { hasAppModifier, isMac } from '../../app/platform';
import type { OsKind } from '../../ipc';
import { focusPaneInDirection, openTerminalSidebar, splitPane } from './api';
import { activePaneController, activePaneId } from './controllers';
import type { PaneDirection } from './workspace';

export type TerminalShortcut =
  | 'copy'
  | 'paste'
  | 'select-all'
  | 'search'
  | 'split-right'
  | 'split-down'
  | `focus-${PaneDirection}`;

const ARROWS: Readonly<Record<string, PaneDirection>> = {
  ArrowLeft: 'left',
  ArrowRight: 'right',
  ArrowUp: 'up',
  ArrowDown: 'down',
};

function altGr(event: KeyboardEvent): boolean {
  return typeof event.getModifierState === 'function' && event.getModifierState('AltGraph');
}

/** The pane focus chord: Ctrl+Alt (Cmd+Option on macOS), without Shift. */
function paneChord(event: KeyboardEvent, os: OsKind | null): boolean {
  if (event.shiftKey || !event.altKey) {
    return false;
  }
  return isMac(os) ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

/** True when the app takes this key while a terminal has focus (the shell does not get it). */
export function isAppShortcut(event: KeyboardEvent, os: OsKind | null): boolean {
  if (altGr(event)) {
    return false;
  }
  if (isMac(os)) {
    return event.metaKey;
  }
  if (event.metaKey) {
    return false;
  }
  if (event.ctrlKey && !event.altKey) {
    if (event.shiftKey) {
      // Every Ctrl+Shift combination (the modifier keys alone are not shortcuts).
      return event.key !== 'Shift' && event.key !== 'Control';
    }
    return /^[1-9]$/.test(event.key) || event.key === ',';
  }
  if (paneChord(event, os) && ARROWS[event.key] !== undefined) {
    return true;
  }
  return event.altKey && !event.ctrlKey && !event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight');
}

/** The terminal shortcut of a key event, or null. */
export function matchTerminalShortcut(event: KeyboardEvent, os: OsKind | null): TerminalShortcut | null {
  if (altGr(event)) {
    return null;
  }
  const direction = ARROWS[event.key];
  if (direction !== undefined && paneChord(event, os)) {
    return `focus-${direction}`;
  }
  if (!event.shiftKey || !hasAppModifier(event, os)) {
    return null;
  }
  // `code` keeps working when Shift changes `key` (or a layout maps it elsewhere).
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  const letter = /^Key[A-Z]$/.test(event.code) ? event.code.slice(3).toLowerCase() : key;
  switch (letter) {
    case 'c':
      return 'copy';
    case 'v':
      return 'paste';
    case 'a':
      return 'select-all';
    case 'f':
      return 'search';
    case 'd':
      return 'split-right';
    case 'e':
      return 'split-down';
    default:
      return null;
  }
}

/** True when keyboard focus is inside a terminal (xterm.js helper textarea). */
export function terminalHasFocus(doc: Document = document): boolean {
  const active = doc.activeElement;
  return active instanceof Element && active.closest('.xterm') !== null;
}

/** Focus is in a modal dialog or a menu: terminal shortcuts stay out of it. */
function inOverlay(doc: Document): boolean {
  const active = doc.activeElement;
  return active instanceof Element && active.closest('[role="dialog"], [role="alertdialog"], [role="menu"]') !== null;
}

/** Runs a terminal shortcut; returns false when it does not apply now. */
export function runTerminalShortcut(shortcut: TerminalShortcut, doc: Document = document): boolean {
  if (activePaneId() === null || inOverlay(doc)) {
    return false;
  }
  const controller = activePaneController();
  switch (shortcut) {
    case 'copy':
    case 'paste':
    case 'select-all':
      // Edit actions act on the focused terminal only (a text field keeps its own).
      if (controller === undefined || !terminalHasFocus(doc)) {
        return false;
      }
      if (shortcut === 'copy') {
        void controller.copySelection();
      } else if (shortcut === 'paste') {
        void controller.pasteFromClipboard();
      } else {
        controller.selectAll();
      }
      return true;
    case 'search':
      openTerminalSidebar('tools');
      // The Tools tab focuses its search field when it opens; focus it again when it is open already.
      queueMicrotask(() => doc.querySelector<HTMLInputElement>('[data-testid="terminal-search-input"]')?.focus());
      return true;
    case 'split-right':
      splitPane('row');
      return true;
    case 'split-down':
      splitPane('column');
      return true;
    default:
      focusPaneInDirection(shortcut.slice('focus-'.length) as PaneDirection);
      return true;
  }
}

/** Installs the terminal shortcuts on `target`; returns a function that removes them. */
export function installTerminalShortcuts(os: OsKind | null, target: Window = window): () => void {
  const handler = (event: KeyboardEvent) => {
    if (event.defaultPrevented) {
      return;
    }
    const shortcut = matchTerminalShortcut(event, os);
    if (shortcut !== null && runTerminalShortcut(shortcut, target.document)) {
      event.preventDefault();
    }
  };
  target.addEventListener('keydown', handler);
  return () => target.removeEventListener('keydown', handler);
}
