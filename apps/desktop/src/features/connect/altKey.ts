// "Hold Alt to open in a new window" (UI 2.4): context menu items do not
// pass their event to `onSelect`, so the state of the Alt key is tracked from
// the keyboard and pointer events that lead to the choice (capture phase, so
// it is known before any handler runs).
//
// Owner: S7.

let altDown = false;
let installed = false;

function track(event: KeyboardEvent | MouseEvent): void {
  altDown = event.altKey;
}

function reset(): void {
  altDown = false;
}

/** Starts tracking (idempotent). */
export function installAltKeyTracker(target: Window = window): void {
  if (installed) {
    return;
  }
  installed = true;
  for (const type of ['keydown', 'keyup', 'pointerdown', 'mousedown', 'click'] as const) {
    target.addEventListener(type, track as EventListener, true);
  }
  target.addEventListener('blur', reset);
}

/** True when Alt was held during the latest keyboard or pointer event. */
export function isAltHeld(): boolean {
  return altDown;
}
