// Sessions merge by `rev` (stage 2 plan, decision 1): older payloads are
// ignored, `closed` removes the session and its pane, and later events of a
// closed session are ignored; watchers and reconnect deadlines.

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SessionInfo } from '../../ipc';
import { mergeSession, reconnectDeadline, wasConnected, watchSession } from './sessionsSync';
import { addTab, INITIAL_WORKSPACE, resetWorkspace, sessionsStore, sessionTab, workspaceStore } from './workspace';

afterEach(() => {
  resetWorkspace();
  vi.useRealTimers();
});

function info(rev: number, patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: 's-1',
    rev,
    kind: 'ssh',
    window: 'w-1',
    state: 'connecting',
    step: 'resolving',
    title: 'web-1',
    terminal_theme: { builtin: 'graphite' },
    backspace: 'default',
    record_history: true,
    ...patch,
  };
}

describe('mergeSession', () => {
  it('keeps the payload with the greatest rev', () => {
    expect(mergeSession(info(2, { step: 'connecting' }))).toBe(true);
    expect(mergeSession(info(1, { step: 'resolving' }))).toBe(false);
    expect(mergeSession(info(2, { step: 'host_key' }))).toBe(false);
    expect(sessionsStore.get().get('s-1')?.step).toBe('connecting');
    expect(mergeSession(info(3, { state: 'connected', step: undefined }))).toBe(true);
    expect(sessionsStore.get().get('s-1')?.state).toBe('connected');
    expect(wasConnected('s-1')).toBe(true);
  });

  it('removes a closed session and its pane, and ignores its later events', () => {
    workspaceStore.set(addTab(INITIAL_WORKSPACE, sessionTab('t-1', 'p-1', 's-1')));
    mergeSession(info(1, { state: 'connected' }));
    mergeSession(info(5, { state: 'closed' }));
    expect(sessionsStore.get().has('s-1')).toBe(false);
    expect(workspaceStore.get().tabs).toEqual([]);
    expect(workspaceStore.get().active).toBe('vaults');
    expect(mergeSession(info(6, { state: 'connected' }))).toBe(false);
    expect(sessionsStore.get().has('s-1')).toBe(false);
  });

  it('notifies watchers with newer states until the session closes', () => {
    const seen: string[] = [];
    const stop = watchSession('s-1', (payload) => seen.push(payload.state));
    mergeSession(info(1));
    mergeSession(info(1));
    mergeSession(info(2, { state: 'connected' }));
    stop();
    mergeSession(info(3, { state: 'dropped' }));
    expect(seen).toEqual(['connecting', 'connected']);
  });

  it('records when the next reconnect attempt starts', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-19T10:00:00.000Z'));
    mergeSession(info(1, { state: 'dropped', reconnect: { attempt: 2, next_in_ms: 2000, waiting_for_unlock: false, in_progress: false } }));
    expect(reconnectDeadline('s-1')).toBe(Date.parse('2026-10-19T10:00:02.000Z'));
    mergeSession(info(2, { state: 'connecting', reconnect: { attempt: 2, waiting_for_unlock: false, in_progress: true } }));
    expect(reconnectDeadline('s-1')).toBeUndefined();
  });
});
