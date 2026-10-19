// Mock known hosts, keys and recent connections (stage 2 plan, sections 4.4,
// 6.3, 6.6). Passphrases are generated at run time; fingerprints are fake.

import { describe, expect, it } from 'vitest';
import { IpcError } from '../errors';
import type { EventMap, EventName, HostView } from '../types';
import { createMockBackend, type MockBackend } from './index';
import { fakeFingerprint, knownHostDisplayAddress } from './connect';

const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

async function unlockedBackend() {
  const backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
  const snapshot = await backend.invoke('state_snapshot', undefined);
  return { backend, vaultId: snapshot.vaults[0]?.vault_id ?? '' };
}

function collect<E extends EventName>(backend: MockBackend, event: E): EventMap[E][] {
  const received: EventMap[E][] = [];
  void backend.listen(event, (payload) => received.push(payload));
  return received;
}

async function rejection(promise: Promise<unknown>): Promise<IpcError> {
  try {
    await promise;
  } catch (reason) {
    expect(reason).toBeInstanceOf(IpcError);
    return reason as IpcError;
  }
  throw new Error('expected the command to fail');
}

function mockKey(options: { passphrase?: string; comment?: string; algorithm?: string; body?: string } = {}): string {
  const lines = ['-----BEGIN OPENSSH PRIVATE KEY-----', options.body ?? crypto.randomUUID()];
  if (options.algorithm !== undefined) {
    lines.push(`mock-algorithm:${options.algorithm}`);
  }
  if (options.comment !== undefined) {
    lines.push(`mock-comment:${options.comment}`);
  }
  if (options.passphrase !== undefined) {
    lines.push(`mock-encrypted:${options.passphrase}`);
  }
  lines.push('-----END OPENSSH PRIVATE KEY-----');
  return lines.join('\n');
}

async function addHost(backend: MockBackend, vaultId: string, data: HostView['data']): Promise<HostView> {
  const view = await backend.invoke('record_save', { vault_id: vaultId, record_id: null, type: 'host', data });
  if (view.type !== 'host') {
    throw new Error('not a host');
  }
  return view;
}

describe('mock known hosts', () => {
  it('lists display addresses, hashed entries last, and matching saved hosts', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const web = await addHost(backend, vaultId, { address: 'web.example.com' });
    const alt = await addHost(backend, vaultId, { address: '192.0.2.8', ssh: { port: 2222 } });
    const control = backend.control.connect;
    control.addKnownHost({ host: '|1|c2FsdA==|aGFzaA==', key_type: 'ssh-ed25519', public_key: 'AAAA1' });
    control.addKnownHost({ host: '192.0.2.8', port: 2222, key_type: 'ecdsa-sha2-nistp256', public_key: 'AAAA2' });
    control.addKnownHost({ host: '*.example.com', key_type: 'ssh-ed25519', public_key: 'AAAA3', revoked: true, comment: 'old' });

    const list = await backend.invoke('known_hosts_list', undefined);
    expect(list.map((entry) => entry.display_address)).toEqual(['*.example.com', '[192.0.2.8]:2222', undefined]);
    const [pattern, ported, hashed] = list;
    expect(pattern).toMatchObject({ revoked: true, comment: 'old', hashed: false, host_ids: [web.record_id] });
    expect(ported).toMatchObject({ port: 2222, key_type: 'ecdsa-sha2-nistp256', host_ids: [alt.record_id], source: 'import' });
    expect(hashed).toMatchObject({ hashed: true, host_ids: [] });
    expect(hashed?.display_address).toBeUndefined();
    expect(ported?.fingerprint).toMatch(/^SHA256:[A-Za-z0-9+/]{43}$/);
  });

  it('removes entries and reports class 8; unknown ids are not_found', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const id = backend.control.connect.addKnownHost({ host: 'db.example.com', key_type: 'ssh-ed25519', public_key: 'AAAA' });
    await flush();
    const events = collect(backend, 'records:changed');
    await backend.invoke('known_host_remove', { vault_id: vaultId, record_ids: [id] });
    await flush();
    expect(await backend.invoke('known_hosts_list', undefined)).toEqual([]);
    expect(events.at(-1)).toMatchObject({ record_ids: [id], classes: [8] });
    const error = await rejection(backend.invoke('known_host_remove', { vault_id: vaultId, record_ids: [id] }));
    expect(error.code).toBe('not_found');
  });

  it('formats addresses like OpenSSH and makes stable fake fingerprints', () => {
    expect(knownHostDisplayAddress('host.example.com', undefined)).toBe('host.example.com');
    expect(knownHostDisplayAddress('host.example.com', 22)).toBe('host.example.com');
    expect(knownHostDisplayAddress('2001:db8::1', 2200)).toBe('[2001:db8::1]:2200');
    expect(knownHostDisplayAddress('|1|a|b', 2200)).toBeUndefined();
    expect(fakeFingerprint('x')).toBe(fakeFingerprint('x'));
    expect(fakeFingerprint('x')).not.toBe(fakeFingerprint('y'));
  });
});

describe('mock keys ("+ -> Key")', () => {
  it('imports an unencrypted key with the comment as the default label', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const result = await backend.invoke('key_import', { vault_id: vaultId, private_key: `\r\n${mockKey({ comment: 'laptop' })}\r\n` });
    expect(result.created).toBe(true);
    expect(result.key).toMatchObject({ label: 'laptop', algorithm: 'ssh-ed25519', encrypted: false, passphrase_saved: false });
    expect(await backend.invoke('keys_list', undefined)).toEqual([result.key]);
  });

  it('asks for the passphrase of an encrypted key and checks it', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const passphrase = `pp-${crypto.randomUUID()}`;
    const text = mockKey({ passphrase });
    const missing = await rejection(backend.invoke('key_import', { vault_id: vaultId, private_key: text }));
    expect(missing.fieldErrors).toEqual([{ field: 'passphrase', reason: 'required' }]);
    const wrong = await rejection(backend.invoke('key_import', { vault_id: vaultId, private_key: text, passphrase: `${passphrase}x` }));
    expect(wrong.fieldErrors).toEqual([{ field: 'passphrase', reason: 'invalid' }]);
    const saved = await backend.invoke('key_import', {
      vault_id: vaultId,
      private_key: text,
      passphrase,
      save_passphrase: true,
      label: 'deploy',
    });
    expect(saved.key).toMatchObject({ label: 'deploy', encrypted: true, passphrase_saved: true });
  });

  it('refuses PEM text, unsupported types, long keys, stray passphrases and long labels', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const cases: [Parameters<typeof backend.invoke<'key_import'>>[1], string, string][] = [
      [{ vault_id: vaultId, private_key: '-----BEGIN RSA PRIVATE KEY-----\nAAAA' }, 'private_key', 'invalid'],
      [{ vault_id: vaultId, private_key: mockKey({ algorithm: 'ssh-dss' }) }, 'private_key', 'not_allowed'],
      [{ vault_id: vaultId, private_key: mockKey({ body: 'A'.repeat(40_000) }) }, 'private_key', 'too_long'],
      [{ vault_id: vaultId, private_key: mockKey(), passphrase: 'unneeded' }, 'passphrase', 'not_allowed'],
      [{ vault_id: vaultId, private_key: mockKey(), label: 'L'.repeat(257) }, 'label', 'too_long'],
    ];
    for (const [input, field, reason] of cases) {
      const error = await rejection(backend.invoke('key_import', input));
      expect(error.fieldErrors, `${field} ${reason}`).toEqual([{ field, reason }]);
    }
    expect((await rejection(backend.invoke('key_import', { vault_id: crypto.randomUUID(), private_key: mockKey() }))).code).toBe(
      'not_found',
    );
  });

  it('returns the existing key for the same fingerprint and emits class 3', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const events = collect(backend, 'records:changed');
    const text = mockKey();
    const first = await backend.invoke('key_import', { vault_id: vaultId, private_key: text });
    const second = await backend.invoke('key_import', { vault_id: vaultId, private_key: text, label: 'other' });
    await flush();
    expect(second).toEqual({ key: first.key, created: false });
    expect(events.filter((event) => event.classes.includes(3))).toHaveLength(1);
  });
});

describe('mock recent connections', () => {
  it('lists the newest connection per host or address with current titles and paths', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const group = await backend.invoke('record_save', { vault_id: vaultId, record_id: null, type: 'group', data: { name: 'prod' } });
    const host = await addHost(backend, vaultId, { address: 'db.example.com', label: 'DB', group_id: group.record_id });
    const control = backend.control.connect;
    const ref = { vault_id: vaultId, record_id: host.record_id };
    control.addConnection({ host: ref, title: 'old title', address: 'db.example.com', port: 22, last_at: '2026-10-01T10:00:00.000Z' });
    control.addConnection({ title: '198.51.100.4', address: '198.51.100.4', port: 2200, username: 'ops', last_at: '2026-10-02T10:00:00.000Z' });
    control.addConnection({ host: ref, title: 'old title', address: 'db.example.com', port: 22, last_at: '2026-10-03T10:00:00.000Z' });

    const rows = await backend.invoke('recent_connections', undefined);
    expect(rows).toEqual([
      { host: ref, title: 'DB', address: 'db.example.com', port: 22, path: ['Personal', 'prod'], last_at: '2026-10-03T10:00:00.000Z' },
      { title: '198.51.100.4', address: '198.51.100.4', port: 2200, username: 'ops', path: ['Personal'], last_at: '2026-10-02T10:00:00.000Z' },
    ]);
  });
});
