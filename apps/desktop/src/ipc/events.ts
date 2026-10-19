// Typed event subscriptions (docs/architecture.md, section 3.5).
//
// Snapshot protocol for a new or reloaded window: subscribe first and buffer,
// then call `stateSnapshot()`, then apply buffered events whose `v` is
// greater than the snapshot's `v` (`createEventBuffer` below).
//
// Owner: W4.

import { getBackend, type Unlisten } from './backend';
import type { EventMap, EventName } from './types';

export type { Unlisten } from './backend';

export function subscribe<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void): Promise<Unlisten> {
  return getBackend().listen(event, handler);
}

/** Broadcast events: every payload carries the state version `v`. */
export const BROADCAST_EVENTS = [
  'lock:changed',
  'records:changed',
  'vaults:changed',
  'settings:changed',
  'notifications:changed',
] as const satisfies readonly EventName[];

export type BroadcastEventName = (typeof BROADCAST_EVENTS)[number];

/**
 * Window-targeted events of stage 2. `sessions:changed` is ordered per
 * session by `rev`, not by `v`, so it is not buffered: a window subscribes
 * before its snapshot and keeps, per session, the payload with the greatest
 * `rev` (snapshot or event).
 */
export const SESSION_EVENTS = ['sessions:changed', 'window:close-requested'] as const satisfies readonly EventName[];

export type BufferedEvent = {
  [E in BroadcastEventName]: { name: E; payload: EventMap[E] };
}[BroadcastEventName];

/**
 * Subscribes to every broadcast event and buffers them until `release(v)` is
 * called with the snapshot version; then forwards buffered events with a
 * greater `v`, and every later event, to `handler`.
 */
export async function createEventBuffer(handler: (event: BufferedEvent) => void): Promise<{
  release: (snapshotVersion: number) => void;
  unlisten: Unlisten;
}> {
  let buffer: BufferedEvent[] | null = [];
  let floor = -1;
  const deliver = (event: BufferedEvent) => {
    if (buffer !== null) {
      buffer.push(event);
    } else if (event.payload.v > floor) {
      handler(event);
    }
  };
  const unlisteners = await Promise.all(
    BROADCAST_EVENTS.map((name) =>
      subscribe(name, (payload) => deliver({ name, payload } as BufferedEvent)),
    ),
  );
  return {
    release(snapshotVersion: number) {
      floor = snapshotVersion;
      const pending = buffer ?? [];
      buffer = null;
      for (const event of pending) {
        if (event.payload.v > floor) {
          handler(event);
        }
      }
    },
    unlisten() {
      for (const unlisten of unlisteners) {
        unlisten();
      }
    },
  };
}
