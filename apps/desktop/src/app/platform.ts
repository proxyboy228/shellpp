// Platform helpers for the UI: the modifier key and shortcut hints
// (UI 0, 1.4: hints are written as symbols such as `Ctrl+,`; Cmd on macOS).
//
// Owner: W4.

import type { OsKind } from '../ipc';

export function isMac(os: OsKind | null | undefined): boolean {
  return os === 'macos';
}

/** A shortcut hint for `Mod+<key>`: `Ctrl+,` or `Cmd+,`. */
export function modShortcut(os: OsKind | null | undefined, key: string): string {
  return `${isMac(os) ? 'Cmd' : 'Ctrl'}+${key}`;
}

/** True when the app modifier (Ctrl, or Cmd on macOS) is the only modifier besides Shift. */
export function hasAppModifier(event: KeyboardEvent, os: OsKind | null | undefined): boolean {
  if (event.altKey) {
    // AltGr (Ctrl+Alt on Windows) types characters; no shortcut fires (UI 12.5).
    return false;
  }
  return isMac(os) ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}
