// Window events of the terminal: `sessions:changed` (per-session `rev`
// ordering) and `window:close-requested` (stage 2 plan, sections 5.3, 6.2).
// Started by the app root next to the window session (src/app/session.ts).
//
// Owner: S6.

import { subscribe, type SessionInfo, type Unlisten, type Uuid } from '../../ipc';
import { requestWindowClose } from './api';
import { startLastSessionWriter } from './lastSession';
import { onWorkspaceReset, removeSessions, sessionsStore, workspaceStore } from './workspace';

/** Sessions that reached `closed`: later (older) events for them are ignored. */
const closedSessions = new Set<Uuid>();
/** Sessions that were `connected` at least once (the pane shows the terminal, not the connection screen). */
const connectedOnce = new Set<Uuid>();
type SessionListener = (info: SessionInfo) => void;
const watchers = new Map<Uuid, Set<SessionListener>>();
/** When the next automatic reconnect attempt starts (ms since the epoch), per dropped session. */
const reconnectDeadlines = new Map<Uuid, number>();

onWorkspaceReset(() => {
  closedSessions.clear();
  connectedOnce.clear();
  watchers.clear();
  reconnectDeadlines.clear();
});

/** The time of the next automatic reconnect attempt of a session, when one is scheduled. */
export function reconnectDeadline(sessionId: Uuid): number | undefined {
  return reconnectDeadlines.get(sessionId);
}

/** True once the session was `connected` (reconnects show the terminal with an overlay). */
export function wasConnected(sessionId: Uuid): boolean {
  return connectedOnce.has(sessionId);
}

/** True when the session reached `closed` in this window. */
export function isClosedSession(sessionId: Uuid): boolean {
  return closedSessions.has(sessionId);
}

/**
 * Calls `listener` with every newer state of a session (S7: the quick connect
 * save offer waits for `connected`). Returns a function that stops.
 */
export function watchSession(sessionId: Uuid, listener: SessionListener): () => void {
  let set = watchers.get(sessionId);
  if (set === undefined) {
    set = new Set();
    watchers.set(sessionId, set);
  }
  set.add(listener);
  return () => {
    const current = watchers.get(sessionId);
    current?.delete(listener);
    if (current !== undefined && current.size === 0) {
      watchers.delete(sessionId);
    }
  };
}

/**
 * Keeps the newest `rev` of each session; `closed` sessions are removed from
 * the store and their panes close (the last pane closes the tab, UI 4.6).
 * Returns true when the payload was applied.
 */
export function mergeSession(info: SessionInfo): boolean {
  if (closedSessions.has(info.session_id)) {
    return false;
  }
  const known = sessionsStore.get().get(info.session_id);
  if (known !== undefined && known.rev >= info.rev) {
    return false;
  }
  // Ended, dropped and reconnecting sessions were connected before (also after a window reload).
  if (info.state === 'connected' || info.state === 'ended' || info.state === 'dropped' || info.reconnect !== undefined) {
    connectedOnce.add(info.session_id);
  }
  // `next_in_ms` counts from the moment the event was emitted; delivery is immediate.
  if (info.reconnect?.next_in_ms !== undefined) {
    reconnectDeadlines.set(info.session_id, Date.now() + info.reconnect.next_in_ms);
  } else {
    reconnectDeadlines.delete(info.session_id);
  }
  sessionsStore.set((current) => {
    const next = new Map(current);
    if (info.state === 'closed') {
      next.delete(info.session_id);
    } else {
      next.set(info.session_id, info);
    }
    return next;
  });
  if (info.state === 'closed') {
    closedSessions.add(info.session_id);
    connectedOnce.delete(info.session_id);
    reconnectDeadlines.delete(info.session_id);
    workspaceStore.set((current) => removeSessions(current, new Set([info.session_id])));
  }
  const listeners = watchers.get(info.session_id);
  if (listeners !== undefined) {
    for (const listener of [...listeners]) {
      listener(info);
    }
    if (info.state === 'closed') {
      watchers.delete(info.session_id);
    }
  }
  return true;
}

/**
 * Subscribes to the window events of the terminal and starts the
 * `session.last` writer; returns a function that stops both.
 */
export function startSessionsSync(): () => void {
  let disposed = false;
  const unlisteners: Unlisten[] = [startLastSessionWriter()];
  const track = (unlisten: Unlisten) => {
    if (disposed) {
      unlisten();
    } else {
      unlisteners.push(unlisten);
    }
  };
  void (async () => {
    try {
      track(await subscribe('sessions:changed', (info) => void mergeSession(info)));
      track(await subscribe('window:close-requested', () => void requestWindowClose().catch(() => undefined)));
    } catch {
      // Without events the window still works from snapshots.
    }
  })();
  return () => {
    disposed = true;
    for (const unlisten of unlisteners.splice(0)) {
      unlisten();
    }
  };
}
