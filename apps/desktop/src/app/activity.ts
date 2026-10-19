// Auto-lock activity (UI 13.3, docs/architecture.md 3.3): keyboard and mouse
// input in this window is reported to the core with `activity()`, at most
// once every 30 seconds. Terminal output never counts.
//
// The core takes the time a report arrives as the time of the input. Input
// inside the 30-second window after a report is therefore not dropped: one
// trailing report is sent when the window ends, so the auto-lock interval
// starts from (at most 30 seconds after) the last input, not from the first.
//
// Owner: W4.

import { activity } from '../ipc';

export const ACTIVITY_INTERVAL_MS = 30_000;
const EVENTS = ['keydown', 'pointerdown', 'pointermove', 'wheel'] as const;

/** Starts reporting input; returns a function that stops it. */
export function installActivityReporter(target: Window = window, intervalMs = ACTIVITY_INTERVAL_MS): () => void {
  let last = Number.NEGATIVE_INFINITY;
  let trailing: ReturnType<typeof setTimeout> | null = null;

  const report = () => {
    last = Date.now();
    activity().catch(() => undefined);
  };
  const cancelTrailing = () => {
    if (trailing !== null) {
      clearTimeout(trailing);
      trailing = null;
    }
  };
  const handler = () => {
    const elapsed = Date.now() - last;
    if (elapsed >= intervalMs) {
      cancelTrailing();
      report();
    } else if (trailing === null) {
      trailing = setTimeout(() => {
        trailing = null;
        report();
      }, intervalMs - elapsed);
    }
  };

  for (const name of EVENTS) {
    target.addEventListener(name, handler, { capture: true, passive: true });
  }
  return () => {
    for (const name of EVENTS) {
      target.removeEventListener(name, handler, { capture: true });
    }
    cancelTrailing();
  };
}
