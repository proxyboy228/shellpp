// Test helpers of the terminal feature: a rendered unlocked app with the mock
// backend, saved hosts the mock connects to without prompts, and access to
// the fake engines (jsdom has no canvas; stage 2 plan, section 9 S6).
// Passwords are generated at run time.
//
// Owner: S6.

import { act, screen, waitFor } from '@testing-library/react';
import { expect } from 'vitest';
import type { HostData, SessionInfo } from '../../../ipc';
import type { MockBackend } from '../../../ipc/mock';
import { mockHostPublicKey } from '../../../ipc/mock/sessions';
import { clearFakeEngines, fakeEngines, type FakeTerminalEngine } from '../engine/fakeEngine';
import { openLocalTerminal, openSession, type OpenResult } from '../api';

export { clearFakeEngines, fakeEngines };

export const secret = () => `pw-${crypto.randomUUID()}`;

/**
 * Adds the personal vault id to a rendered app (`renderUnlocked` of
 * src/test/app.tsx, called by the test so that this module stays out of the
 * test-only type set) and forgets the fake engines of earlier tests.
 */
export async function withVault<T extends { backend: MockBackend }>(rendered: Promise<T>): Promise<T & { vaultId: string }> {
  clearFakeEngines();
  const value = await rendered;
  const snapshot = await value.backend.invoke('state_snapshot', undefined);
  return { ...value, vaultId: snapshot.vaults[0]?.vault_id as string };
}

/**
 * A saved host with a known host key and (unless `withPassword` is false) a
 * saved password: connecting asks nothing.
 */
export async function savedHost(
  backend: MockBackend,
  vaultId: string,
  data: Partial<HostData> = {},
  withPassword = true,
): Promise<string> {
  const address = data.address ?? `192.0.2.${10 + Math.floor(Math.random() * 200)}`;
  await backend.invoke('e2e_mock', {
    kind: 'known_host_add',
    host: address,
    key_type: 'ssh-ed25519',
    public_key: mockHostPublicKey(address, data.ssh?.port ?? 22),
  });
  const view = await backend.invoke('record_save', {
    vault_id: vaultId,
    type: 'host',
    data: { label: 'web-1', ssh: { username: 'alice' }, ...data, address },
    ...(withPassword ? { secrets: { 'ssh.password': secret() } } : {}),
  });
  return view.record_id;
}

export function sessionOf(backend: MockBackend, sessionId: string): SessionInfo | undefined {
  return backend.control.sessions.list().find((session) => session.session_id === sessionId);
}

/** The fake engine of a session's terminal view, once it is mounted. */
export async function engineFor(sessionId: string): Promise<FakeTerminalEngine> {
  let engine: FakeTerminalEngine | undefined;
  await waitFor(() => {
    const views = screen.getAllByTestId('terminal-view').filter((view) => view.getAttribute('data-session-id') === sessionId);
    expect(views).toHaveLength(1);
    engine = fakeEngines().find((candidate) => !candidate.disposed && views[0]?.contains(candidate.container ?? null));
    expect(engine).toBeDefined();
  });
  return engine as FakeTerminalEngine;
}

/** Types text one character at a time, waiting for the shell's echo (as a person types). */
export async function typeLine(engine: FakeTerminalEngine, text: string, enter = true): Promise<void> {
  for (const char of text) {
    const before = engine.written.length;
    act(() => engine.type(char));
    await waitFor(() => expect(engine.written.length).toBeGreaterThan(before));
  }
  if (enter) {
    const before = engine.written.length;
    act(() => engine.type('\r'));
    await waitFor(() => expect(engine.written.length).toBeGreaterThan(before));
  }
}

export async function openLocal(): Promise<OpenResult> {
  let result: OpenResult | null = null;
  await act(async () => {
    result = await openLocalTerminal();
  });
  expect(result).not.toBeNull();
  return result as unknown as OpenResult;
}

export async function openHost(vaultId: string, recordId: string): Promise<OpenResult> {
  let result: OpenResult | null = null;
  await act(async () => {
    result = await openSession({ kind: 'host', vault_id: vaultId, record_id: recordId });
  });
  expect(result).not.toBeNull();
  return result as unknown as OpenResult;
}

/** Waits until the mock session reaches a state. */
export async function waitForState(backend: MockBackend, sessionId: string, state: SessionInfo['state']): Promise<void> {
  await waitFor(() => expect(sessionOf(backend, sessionId)?.state).toBe(state));
}
