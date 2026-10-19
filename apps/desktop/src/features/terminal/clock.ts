// A wall clock for countdowns (reconnect, UI 4.6), outside React so that
// components stay pure. It ticks once a second, only while subscribed.
//
// Owner: S6.

import { useSyncExternalStore } from 'react';

const TICK_MS = 1000;

let now = Date.now();
let timer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (timer === null) {
    now = Date.now();
    timer = setInterval(() => {
      now = Date.now();
      for (const current of [...listeners]) {
        current();
      }
    }, TICK_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
}

function getSnapshot(): number {
  // Not ticking yet (first render): refresh a stale value, but keep it stable within a second.
  if (timer === null && Date.now() - now >= TICK_MS) {
    now = Date.now();
  }
  return now;
}

/** Milliseconds since the epoch; re-renders once a second. */
export function useClock(): number {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
