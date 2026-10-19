// Connect helpers (UI 2.1, 2.4): one session per host, new windows with Alt,
// the "connected" watch behind the quick-connect save offer, and the saved
// host data.

import { act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionInfo } from '../../ipc';
import { clearToasts, getToasts } from '../../app/toast';
import { handleWindowIntent, mergeSession, resetWorkspace, sessionsStore } from '../terminal';
import { connectHost, connectHosts, onSessionConnected, quickHostData } from './actions';
import { openSession } from './workspaceBridge';

vi.mock('./workspaceBridge', () => ({
  openSession: vi.fn(() => Promise.resolve(null)),
  openLocalTerminal: vi.fn(() => Promise.resolve(null)),
  activateTab: vi.fn(),
  restoreLastSession: vi.fn(() => Promise.resolve()),
}));

const VAULT = '01990000-0000-7000-8000-00000000aaaa';

function info(state: SessionInfo['state'], rev: number, patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: 's-1',
    rev,
    kind: 'ssh',
    window: 'w-1',
    state,
    title: '192.0.2.70',
    terminal_theme: { builtin: 'graphite' },
    backspace: 'default',
    record_history: false,
    ...patch,
  };
}

beforeEach(() => {
  resetWorkspace();
  vi.mocked(openSession).mockClear();
});

afterEach(() => {
  resetWorkspace();
});

describe('connect actions', () => {
  it('opens one session per host, passing the new-window choice', async () => {
    await connectHost({ vault_id: VAULT, record_id: 'h-1' }, { newWindow: true });
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'host', vault_id: VAULT, record_id: 'h-1' }, { newWindow: true });
    await connectHosts(
      [
        { vault_id: VAULT, record_id: 'h-2' },
        { vault_id: VAULT, record_id: 'h-3' },
      ],
      { newWindow: false },
    );
    expect(vi.mocked(openSession).mock.calls.slice(1).map((call) => call[0])).toEqual([
      { kind: 'host', vault_id: VAULT, record_id: 'h-2' },
      { kind: 'host', vault_id: VAULT, record_id: 'h-3' },
    ]);
  });

  it('reports a session that is already connected at once', () => {
    act(() => sessionsStore.set(new Map([['s-1', info('connected', 1)]])));
    const listener = vi.fn();
    onSessionConnected('s-1', listener);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('reports a session once when it connects, also after a failed attempt', () => {
    const listener = vi.fn();
    onSessionConnected('s-1', listener);
    act(() => void mergeSession(info('connecting', 1)));
    act(() => void mergeSession(info('failed', 2)));
    expect(listener).not.toHaveBeenCalled();
    act(() => void mergeSession(info('connected', 3)));
    act(() => void mergeSession(info('connected', 4)));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener.mock.calls[0]?.[0]).toMatchObject({ rev: 3 });
  });

  it('stops watching when the session closes or when stopped', () => {
    const closed = vi.fn();
    onSessionConnected('s-1', closed);
    act(() => void mergeSession(info('connecting', 1)));
    act(() => void mergeSession(info('closed', 2)));
    expect(closed).not.toHaveBeenCalled();

    const stopped = vi.fn();
    const stop = onSessionConnected('s-2', stopped);
    stop();
    act(() => void mergeSession(info('connected', 1, { session_id: 's-2' })));
    expect(stopped).not.toHaveBeenCalled();
  });

  it('offers to save a quick connect host in the new window that opened it (Alt)', async () => {
    clearToasts();
    vi.mocked(openSession).mockResolvedValueOnce({ tabId: 't-1', paneId: 'p-1', sessionId: 's-1' });
    await act(async () => {
      handleWindowIntent({ kind: 'open_session', target: { kind: 'quick', address: '192.0.2.71', port: 2200 } });
      await Promise.resolve();
    });
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'quick', address: '192.0.2.71', port: 2200 }, {});
    act(() => void mergeSession(info('connected', 1)));
    expect(getToasts().some((toast) => toast.testId === 'hosts-quick-connect-save-toast')).toBe(true);
    clearToasts();
  });

  it('builds the saved host from the target and what the session used', () => {
    expect(quickHostData({ address: '192.0.2.70' })).toEqual({ address: '192.0.2.70' });
    expect(quickHostData({ address: '192.0.2.70', port: 22, username: 'root' })).toEqual({ address: '192.0.2.70', ssh: { username: 'root' } });
    expect(quickHostData({ address: '192.0.2.70' }, info('connected', 1, { port: 2201, username: 'asked' }))).toEqual({
      address: '192.0.2.70',
      ssh: { port: 2201, username: 'asked' },
    });
  });
});
