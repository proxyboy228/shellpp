// Mock sessions, local terminal and history (stage 2 plan, section 6.6):
// connect steps and prompts, TOFU and changed or revoked keys, the echo
// shell contract, exit codes, drops with reconnect attempts, lock, history.
// Passwords are generated at run time.

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { IpcError } from '../errors';
import type { PromptReply, SessionInfo, SessionPrompt, SessionTarget } from '../types';
import { createMockBackend, type MockBackend } from './index';
import { mockFingerprint, mockHostPublicKey } from './sessions';

afterEach(() => {
  vi.useRealTimers();
});

const secret = () => `pw-${crypto.randomUUID()}`;

async function setup(options: Parameters<typeof createMockBackend>[0] = {}) {
  const password = secret();
  const backend = createMockBackend({ initialState: 'unlocked', password, ...options });
  const snapshot = await backend.invoke('state_snapshot', undefined);
  const vaultId = snapshot.vaults[0]?.vault_id as string;
  return { backend, vaultId, password };
}

async function saveHost(backend: MockBackend, vaultId: string, data: Record<string, unknown>, hostPassword?: string) {
  const view = await backend.invoke('record_save', {
    vault_id: vaultId,
    type: 'host',
    data: { address: '192.0.2.10', ...data },
    ...(hostPassword === undefined ? {} : { secrets: { 'ssh.password': hostPassword } }),
  });
  return view.record_id;
}

function current(backend: MockBackend, id: string): SessionInfo | undefined {
  return backend.control.sessions.list().find((session) => session.session_id === id);
}

async function until(backend: MockBackend, id: string, check: (info: SessionInfo | undefined) => boolean) {
  await vi.waitFor(() => expect(check(current(backend, id))).toBe(true));
  return current(backend, id);
}

async function prompt(backend: MockBackend, id: string, kind: SessionPrompt['kind']): Promise<SessionPrompt> {
  const info = await until(backend, id, (value) => value?.prompt?.kind === kind);
  return info?.prompt as SessionPrompt;
}

async function reply(backend: MockBackend, id: string, kind: SessionPrompt['kind'], answer: PromptReply) {
  const asked = await prompt(backend, id, kind);
  await backend.invoke('session_prompt_reply', { session_id: id, prompt_id: asked.prompt_id, reply: answer });
  return asked;
}

function attach(backend: MockBackend, id: string) {
  const decoder = new TextDecoder();
  let text = '';
  const channel = backend.createChannel<ArrayBuffer>((buffer) => {
    text += decoder.decode(new Uint8Array(buffer));
  });
  void backend.invoke('session_attach', { session_id: id, on_output: channel });
  return { text: () => text };
}

async function open(backend: MockBackend, target: SessionTarget) {
  return backend.invoke('session_open', { target, cols: 80, rows: 24 });
}

describe('mock sessions', () => {
  it('connects to a saved host: TOFU, accept and save, echo shell output through the channel', async () => {
    const { backend, vaultId } = await setup();
    const events: SessionInfo[] = [];
    await backend.listen('sessions:changed', (info) => events.push(info));
    const recordId = await saveHost(backend, vaultId, { label: 'web-1', ssh: { username: 'alice' } }, secret());
    const opened = await open(backend, { kind: 'host', vault_id: vaultId, record_id: recordId });
    expect(opened).toMatchObject({ state: 'connecting', step: 'resolving', rev: 1, title: 'web-1', port: 22, username: 'alice', window: 'w-1' });

    const asked = await reply(backend, opened.session_id, 'host_key_unknown', { kind: 'host_key', decision: 'accept_save' });
    expect(asked).toMatchObject({
      key: { address: '192.0.2.10', port: 22, key_type: 'ssh-ed25519', fingerprint: mockFingerprint(mockHostPublicKey('192.0.2.10')) },
    });
    await until(backend, opened.session_id, (info) => info?.state === 'connected');
    const output = attach(backend, opened.session_id);
    await vi.waitFor(() => expect(output.text()).toBe('Welcome to shellpp-test-sshd\r\nalice@shellpp-test:~$ '));

    await backend.invoke('session_write', { session_id: opened.session_id, data: 'echo hi\r' });
    await vi.waitFor(() => expect(output.text()).toContain('echo hi\r\nhi\r\nalice@shellpp-test:~$ '));
    expect(backend.control.sessions.input(opened.session_id)).toBe('echo hi\r');

    // Revisions only grow; the steps are reported in order.
    const revs = events.map((info) => info.rev);
    expect([...revs].sort((a, b) => a - b)).toEqual(revs);
    expect(events.map((info) => info.step).filter(Boolean)).toEqual(
      expect.arrayContaining(['resolving', 'connecting', 'host_key', 'authenticating', 'opening_session']),
    );

    // The key is known now: the next connection does not ask.
    const second = await open(backend, { kind: 'host', vault_id: vaultId, record_id: recordId });
    await until(backend, second.session_id, (info) => info?.state === 'connected');
    expect(events.some((info) => info.session_id === second.session_id && info.prompt !== undefined)).toBe(false);
    const recent = await backend.invoke('recent_connections', undefined);
    void recent; // S7's handler lists `ctx.connections`.
  });

  it('asks for the username and password of a quick connect; a wrong password shows the error', async () => {
    const { backend } = await setup();
    const opened = await open(backend, { kind: 'quick', address: '198.51.100.7', port: 2222 });
    expect(opened).toMatchObject({ title: '198.51.100.7', port: 2222 });
    await reply(backend, opened.session_id, 'host_key_unknown', { kind: 'host_key', decision: 'connect_once' });
    await reply(backend, opened.session_id, 'username', { kind: 'text', value: 'bob' });
    const first = await reply(backend, opened.session_id, 'password', { kind: 'text', value: `wrong-${secret()}` });
    expect(first).toMatchObject({ kind: 'password', username: 'bob', error: false, can_save: false });
    await vi.waitFor(() => expect(current(backend, opened.session_id)?.prompt).toMatchObject({ kind: 'password', error: true }));
    await reply(backend, opened.session_id, 'password', { kind: 'text', value: secret() });
    const connected = await until(backend, opened.session_id, (info) => info?.state === 'connected');
    expect(connected?.username).toBe('bob');

    // Exit codes: non-zero ends the session; Reconnect connects again with the kept password.
    await backend.invoke('session_write', { session_id: opened.session_id, data: 'exit 3\r' });
    const ended = await until(backend, opened.session_id, (info) => info?.state === 'ended');
    expect(ended?.exit_status).toBe(3);
    const again = await backend.invoke('session_reconnect', { session_id: opened.session_id });
    expect(again.state).toBe('connecting');
    await until(backend, opened.session_id, (info) => info?.state === 'connected');
    await backend.invoke('session_write', { session_id: opened.session_id, data: 'exit\r' });
    await vi.waitFor(() => expect(current(backend, opened.session_id)).toBeUndefined());
    expect((await backend.invoke('app_exit', undefined)).needs_confirmation).toBe(false);
  });

  it('saves a prompted password on the host when Save is checked', async () => {
    const { backend, vaultId } = await setup();
    const recordId = await saveHost(backend, vaultId, { ssh: { username: 'carol' } });
    const opened = await open(backend, { kind: 'host', vault_id: vaultId, record_id: recordId });
    await reply(backend, opened.session_id, 'host_key_unknown', { kind: 'host_key', decision: 'accept_save' });
    const value = secret();
    const asked = await reply(backend, opened.session_id, 'password', { kind: 'text', value, save: true });
    expect(asked).toMatchObject({ can_save: true });
    await until(backend, opened.session_id, (info) => info?.state === 'connected');
    expect((await backend.invoke('secret_reveal', { vault_id: vaultId, record_id: recordId, path: 'ssh.password' })).value).toBe(value);
  });

  it('cancels during the host key step and on a prompt', async () => {
    const { backend } = await setup();
    const opened = await open(backend, { kind: 'quick', address: '203.0.113.5', username: 'dave' });
    await prompt(backend, opened.session_id, 'host_key_unknown');
    await backend.invoke('session_cancel', { session_id: opened.session_id });
    expect(current(backend, opened.session_id)).toMatchObject({ state: 'failed', error: { code: 'cancelled' } });
    expect(current(backend, opened.session_id)?.prompt).toBeUndefined();

    await backend.invoke('session_reconnect', { session_id: opened.session_id });
    await reply(backend, opened.session_id, 'host_key_unknown', { kind: 'host_key', decision: 'cancel' });
    await vi.waitFor(() =>
      expect(current(backend, opened.session_id)).toMatchObject({ state: 'failed', error: { code: 'ssh_error', reason: 'host_key_rejected' } }),
    );

    await backend.invoke('session_reconnect', { session_id: opened.session_id });
    const asked = await prompt(backend, opened.session_id, 'host_key_unknown');
    const wrongKind = await backend
      .invoke('session_prompt_reply', { session_id: opened.session_id, prompt_id: asked.prompt_id, reply: { kind: 'text', value: 'x' } })
      .catch((error: IpcError) => error);
    expect((wrongKind as IpcError).code).toBe('validation_failed');
    const wrongId = await backend
      .invoke('session_prompt_reply', { session_id: opened.session_id, prompt_id: asked.prompt_id + 99, reply: { kind: 'cancel' } })
      .catch((error: IpcError) => error);
    expect((wrongId as IpcError).code).toBe('not_found');
    await backend.invoke('session_prompt_reply', { session_id: opened.session_id, prompt_id: asked.prompt_id, reply: { kind: 'cancel' } });
    expect(current(backend, opened.session_id)).toMatchObject({ state: 'failed', error: { code: 'cancelled' } });
    await expect(backend.invoke('session_reconnect', { session_id: crypto.randomUUID() })).rejects.toMatchObject({ code: 'not_found' });
  });

  it('blocks a changed key until it is replaced, and a revoked key for good', async () => {
    const { backend } = await setup();
    const target: SessionTarget = { kind: 'quick', address: '192.0.2.44', username: 'erin' };
    const first = await open(backend, target);
    await reply(backend, first.session_id, 'host_key_unknown', { kind: 'host_key', decision: 'accept_save' });
    await reply(backend, first.session_id, 'password', { kind: 'text', value: secret() });
    await until(backend, first.session_id, (info) => info?.state === 'connected');

    backend.control.sessions.rotateHostKey('192.0.2.44');
    const second = await open(backend, target);
    const changed = await prompt(backend, second.session_id, 'host_key_changed');
    expect(changed).toMatchObject({
      old_key_type: 'ssh-ed25519',
      old_fingerprint: mockFingerprint(mockHostPublicKey('192.0.2.44')),
      key: { fingerprint: mockFingerprint(mockHostPublicKey('192.0.2.44', 22, 1)) },
    });
    await backend.invoke('session_prompt_reply', { session_id: second.session_id, prompt_id: changed.prompt_id, reply: { kind: 'host_key', decision: 'replace' } });
    await reply(backend, second.session_id, 'password', { kind: 'text', value: secret() });
    await until(backend, second.session_id, (info) => info?.state === 'connected');

    await backend.invoke('e2e_mock', {
      kind: 'known_host_add',
      host: '192.0.2.99',
      key_type: 'ssh-ed25519',
      public_key: mockHostPublicKey('192.0.2.99'),
      revoked: true,
    });
    const revoked = await open(backend, { kind: 'quick', address: '192.0.2.99' });
    const failed = await until(backend, revoked.session_id, (info) => info?.state === 'failed');
    expect(failed).toMatchObject({ error: { code: 'host_key_revoked' }, host_key: { address: '192.0.2.99', key_type: 'ssh-ed25519' } });
  });

  it('drops, schedules reconnect attempts with backoff, and Cancel stops them', async () => {
    vi.useFakeTimers();
    const { backend } = await setup();
    const opened = await open(backend, { kind: 'quick', address: '192.0.2.20', username: 'fay' });
    await vi.advanceTimersByTimeAsync(10);
    let info = current(backend, opened.session_id);
    await backend.invoke('session_prompt_reply', { session_id: opened.session_id, prompt_id: info?.prompt?.prompt_id as number, reply: { kind: 'host_key', decision: 'accept_save' } });
    await vi.advanceTimersByTimeAsync(10);
    info = current(backend, opened.session_id);
    await backend.invoke('session_prompt_reply', { session_id: opened.session_id, prompt_id: info?.prompt?.prompt_id as number, reply: { kind: 'text', value: secret() } });
    await vi.advanceTimersByTimeAsync(10);
    expect(current(backend, opened.session_id)?.state).toBe('connected');

    await backend.invoke('session_write', { session_id: opened.session_id, data: 'drop\r' });
    expect(current(backend, opened.session_id)).toMatchObject({
      state: 'dropped',
      error: { code: 'ssh_error', reason: 'disconnected' },
      reconnect: { attempt: 1, next_in_ms: 1000, waiting_for_unlock: false, in_progress: false },
    });
    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(10);
    expect(current(backend, opened.session_id)?.state).toBe('connected');
    expect(current(backend, opened.session_id)?.reconnect).toBeUndefined();

    backend.control.sessions.setRefusing(true);
    backend.control.sessions.drop(opened.session_id);
    await vi.advanceTimersByTimeAsync(1010);
    expect(current(backend, opened.session_id)).toMatchObject({
      state: 'dropped',
      error: { code: 'ssh_error', reason: 'refused' },
      reconnect: { attempt: 2, next_in_ms: 2000 },
    });
    await backend.invoke('session_cancel', { session_id: opened.session_id });
    expect(current(backend, opened.session_id)?.reconnect).toBeUndefined();
    await vi.advanceTimersByTimeAsync(5000);
    expect(current(backend, opened.session_id)?.state).toBe('dropped');

    backend.control.sessions.setRefusing(false);
    await backend.invoke('session_reconnect', { session_id: opened.session_id });
    await vi.advanceTimersByTimeAsync(10);
    expect(current(backend, opened.session_id)?.state).toBe('connected');
  });

  it('waits for unlock before reconnecting', async () => {
    vi.useFakeTimers();
    const { backend, password } = await setup();
    const opened = await backend.invoke('local_open', { cols: 80, rows: 24 });
    expect(opened).toMatchObject({ kind: 'local', state: 'connected', title: 'PowerShell', shell_id: 'powershell' });
    const ssh = await open(backend, { kind: 'quick', address: '192.0.2.21', username: 'gus' });
    await vi.advanceTimersByTimeAsync(10);
    await backend.invoke('session_prompt_reply', {
      session_id: ssh.session_id,
      prompt_id: current(backend, ssh.session_id)?.prompt?.prompt_id as number,
      reply: { kind: 'host_key', decision: 'accept_save' },
    });
    await vi.advanceTimersByTimeAsync(10);
    await backend.invoke('session_prompt_reply', {
      session_id: ssh.session_id,
      prompt_id: current(backend, ssh.session_id)?.prompt?.prompt_id as number,
      reply: { kind: 'text', value: secret() },
    });
    await vi.advanceTimersByTimeAsync(10);
    await backend.invoke('lock', undefined);
    backend.control.sessions.drop(ssh.session_id);
    expect(current(backend, ssh.session_id)?.reconnect).toEqual({ attempt: 1, waiting_for_unlock: true, in_progress: false });
    await vi.advanceTimersByTimeAsync(5000);
    expect(current(backend, ssh.session_id)?.state).toBe('dropped');
    await backend.invoke('unlock', { password });
    await vi.advanceTimersByTimeAsync(10);
    // The kept password was wiped on lock: the attempt asks again.
    expect(current(backend, ssh.session_id)?.prompt?.kind).toBe('password');
    expect((await backend.invoke('app_exit', undefined)).active_sessions).toBe(2);
  });

  it('records typed history candidates that pass the detector, never injected lines', async () => {
    const { backend, vaultId } = await setup();
    const local = await backend.invoke('local_open', { shell_id: 'cmd', cols: 80, rows: 24 });
    const write = (data: string, command: string, extra: Partial<{ prompt_text: string; alt_screen: boolean }> = {}) =>
      backend.invoke('session_write', {
        session_id: local.session_id,
        data,
        history: { command, prompt_text: extra.prompt_text ?? 'C:\\mock>', typed_printable: true, alt_screen: extra.alt_screen ?? false },
      });
    await write('\r', 'echo one');
    await write('\r', 'echo two');
    await write('\r', 'export GITHUB_TOKEN=abc');
    await write('\r', 'hunter2', { prompt_text: '[sudo] password for test:' });
    await write('\r', 'vim notes', { alt_screen: true });
    await backend.invoke('session_inject', { session_id: local.session_id, data: 'echo injected' });
    await write('\r', 'echo injected');
    await write('\r', 'echo three');
    const list = await backend.invoke('history_list', {});
    expect(list.map((entry) => entry.command)).toEqual(['echo three', 'echo two', 'echo one']);
    expect(await backend.invoke('history_list', { text: 'TWO' })).toHaveLength(1);
    expect(await backend.invoke('history_list', { host: { vault_id: vaultId, record_id: crypto.randomUUID() } })).toEqual([]);

    await backend.invoke('history_delete', { vault_id: vaultId, record_ids: [list[0]?.record_id as string] });
    expect((await backend.invoke('history_list', {})).map((entry) => entry.command)).toEqual(['echo two', 'echo one']);
    await expect(backend.invoke('history_delete', { vault_id: vaultId, record_ids: [crypto.randomUUID()] })).rejects.toMatchObject({ code: 'not_found' });
    expect(await backend.invoke('history_clear', {})).toEqual({ removed: 2 });
  });

  it('validates inputs and runs the echo shell commands', async () => {
    const { backend, vaultId } = await setup();
    await expect(backend.invoke('session_open', { target: { kind: 'quick', address: ' ' }, cols: 80, rows: 24 })).rejects.toMatchObject({
      code: 'validation_failed',
      details: { errors: [{ field: 'address', reason: 'required' }] },
    });
    await expect(backend.invoke('session_open', { target: { kind: 'quick', address: 'example.com' }, cols: 0, rows: 24 })).rejects.toMatchObject({
      details: { errors: [{ field: 'cols', reason: 'out_of_range' }] },
    });
    await expect(open(backend, { kind: 'host', vault_id: vaultId, record_id: crypto.randomUUID() })).rejects.toMatchObject({ code: 'not_found' });
    await expect(backend.invoke('local_open', { shell_id: 'nope', cols: 80, rows: 24 })).rejects.toMatchObject({
      details: { errors: [{ field: 'shell_id', reason: 'invalid' }] },
    });
    expect(await backend.invoke('local_shells', undefined)).toEqual({
      shells: [
        { id: 'powershell', label: 'PowerShell' },
        { id: 'cmd', label: 'Command Prompt' },
      ],
      default_id: 'powershell',
    });

    const local = await backend.invoke('local_open', { cols: 80, rows: 24 });
    const output = attach(backend, local.session_id);
    await backend.invoke('session_resize', { session_id: local.session_id, cols: 120, rows: 40 });
    for (const line of ['size', 'term', 'lines 3', 'nope', 'read-secret']) {
      await backend.invoke('session_write', { session_id: local.session_id, data: `${line}\r` });
    }
    await backend.invoke('session_write', { session_id: local.session_id, data: 'hidden\r' });
    await vi.waitFor(() => expect(output.text()).toContain('[sudo] password for test: \r\nok\r\n'));
    const text = output.text();
    expect(text).toContain('120x40\r\n');
    expect(text).toContain('xterm-256color\r\n');
    expect(text).toContain('line 1\r\nline 2\r\nline 3\r\n');
    expect(text).toContain('nope: command not found\r\n');
    expect(text).not.toContain('hidden');

    // Backspace erases; Ctrl+C prints ^C; alt-screen waits for q.
    await backend.invoke('session_write', { session_id: local.session_id, data: 'ab\x7f' });
    await backend.invoke('session_write', { session_id: local.session_id, data: '\x03alt-screen\r' });
    await backend.invoke('session_write', { session_id: local.session_id, data: 'xq' });
    await vi.waitFor(() => expect(output.text()).toContain('\x1b[?1049l'));
    expect(output.text()).toContain('ab\x08 \x08^C\r\n');

    // Input to an ended session is dropped; Close removes it.
    await backend.invoke('session_write', { session_id: local.session_id, data: 'exit 2\r' });
    expect(current(backend, local.session_id)).toMatchObject({ state: 'ended', exit_status: 2 });
    await backend.invoke('session_write', { session_id: local.session_id, data: 'echo late\r' });
    expect(backend.control.sessions.input(local.session_id)).not.toContain('late');
    await backend.invoke('session_close', { session_id: local.session_id });
    expect(current(backend, local.session_id)).toBeUndefined();
    expect((await backend.invoke('state_snapshot', undefined)).sessions).toEqual([]);
  });
});
