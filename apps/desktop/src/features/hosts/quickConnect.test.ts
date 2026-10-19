import { describe, expect, it } from 'vitest';
import type { HostView } from '../../ipc';
import { findExactHost, parseQuickConnect, resolveSearchConnect } from './quickConnect';

describe('parseQuickConnect (UI 2.1, D23)', () => {
  it.each([
    ['db1.example.com', { address: 'db1.example.com' }],
    ['192.0.2.4', { address: '192.0.2.4' }],
    ['localhost', { address: 'localhost' }],
    ['2001:db8::5', { address: '2001:db8::5' }],
    ['admin@web', { address: 'web', username: 'admin' }],
    ['admin@web.example.org', { address: 'web.example.org', username: 'admin' }],
    ['admin@web.example.org:2222', { address: 'web.example.org', username: 'admin', port: 2222 }],
    ['root@[2001:db8::7]:2200', { address: '2001:db8::7', username: 'root', port: 2200 }],
    ['ssh admin@198.51.100.3 -p 2022', { address: '198.51.100.3', username: 'admin', port: 2022 }],
    ['ssh -p 2022 admin@198.51.100.3', { address: '198.51.100.3', username: 'admin', port: 2022 }],
    ['ssh backup', { address: 'backup' }],
  ])('recognizes %j', (text, target) => {
    expect(parseQuickConnect(text)).toEqual(target);
  });

  it.each([
    [''],
    ['prod'],
    ['web server'],
    ['admin@'],
    ['@host.example.com'],
    ['admin@web.example.org:99999'],
    ['ssh admin@web.example.org -p'],
    ['ssh -v admin@web.example.org'],
    ['ssh admin@a.example.com admin@b.example.com'],
    ['admin@web.example.org:22 extra'],
  ])('treats %j as a search query', (text) => {
    expect(parseQuickConnect(text)).toBeNull();
  });
});

function host(recordId: string, address: string, label?: string): HostView {
  return {
    type: 'host',
    vault_id: '01990000-0000-7000-8000-00000000aaaa',
    record_id: recordId,
    revision: 1,
    created_at: '2026-10-01T00:00:00.000Z',
    modified_at: '2026-10-01T00:00:00.000Z',
    inherited: {},
    path: [],
    data: label === undefined ? { address } : { address, label },
    secrets_set: [],
  };
}

describe('resolveSearchConnect (UI 2.1: exact match, address, search)', () => {
  const hosts = [host('h-1', 'db1.example.com', 'Primary DB'), host('h-2', '192.0.2.10'), host('h-3', 'web.example.org', 'prod')];

  it('connects to a saved host whose label or address matches exactly, ignoring case', () => {
    expect(resolveSearchConnect('primary db', hosts)).toEqual({ kind: 'host', host: hosts[0] });
    expect(resolveSearchConnect('  DB1.EXAMPLE.COM ', hosts)).toEqual({ kind: 'host', host: hosts[0] });
    expect(resolveSearchConnect('192.0.2.10', hosts)).toEqual({ kind: 'host', host: hosts[1] });
    // A single word that is no address still connects when it is a saved label.
    expect(resolveSearchConnect('prod', hosts)).toEqual({ kind: 'host', host: hosts[2] });
  });

  it('quick connects to an address that is not a saved host', () => {
    expect(resolveSearchConnect('admin@198.51.100.7:2200', hosts)).toEqual({
      kind: 'quick',
      target: { address: '198.51.100.7', username: 'admin', port: 2200 },
    });
    // `user@address` of a saved address is still a quick connect (not an exact match).
    expect(resolveSearchConnect('ops@db1.example.com', hosts)).toEqual({
      kind: 'quick',
      target: { address: 'db1.example.com', username: 'ops' },
    });
  });

  it('is null for a plain search query (Connect disabled)', () => {
    expect(resolveSearchConnect('primary', hosts)).toBeNull();
    expect(resolveSearchConnect('', hosts)).toBeNull();
    expect(resolveSearchConnect('web server', hosts)).toBeNull();
  });

  it('prefers a label match over an address match', () => {
    const list = [host('h-a', 'alpha.example.com'), host('h-b', 'beta.example.com', 'alpha.example.com')];
    expect(findExactHost('alpha.example.com', list)?.record_id).toBe('h-b');
    expect(findExactHost('gamma', list)).toBeUndefined();
  });
});
