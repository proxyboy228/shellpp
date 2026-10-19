// Dialogs opened from the Settings window are modals inside a modal. Their
// key events must not reach the Settings modal through React's tree (a Tab
// there would move focus back into the Settings window).
//
// Owner: W4.

import type { KeyboardEvent } from 'react';

export function stopKeyPropagation(event: KeyboardEvent): void {
  event.stopPropagation();
}
