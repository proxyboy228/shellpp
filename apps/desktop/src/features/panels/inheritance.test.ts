import { describe, expect, it } from 'vitest';
import type { GroupSsh, GroupView, Uuid } from '../../ipc';
import { inheritanceChain, inheritedPlaceholder, resolveInherited } from './inheritance';

function group(id: Uuid, name: string, path: Uuid[], ssh: GroupSsh = {}): GroupView {
  return {
    type: 'group',
    vault_id: 'v',
    record_id: id,
    revision: 1,
    created_at: '2026-10-18T00:00:00.000Z',
    modified_at: '2026-10-18T00:00:00.000Z',
    inherited: {},
    path,
    data: { name, ssh },
    host_count: 0,
    child_count: 0,
  };
}

// prod (port 2222, user deploy) > web (user www) > edge; lone (no values).
const prod = group('g-prod', 'prod', [], { port: 2222, username: 'deploy' });
const web = group('g-web', 'web', ['g-prod'], { username: 'www' });
const edge = group('g-edge', 'edge', ['g-prod', 'g-web']);
const lone = group('g-lone', 'lone', []);
const byId = new Map([prod, web, edge, lone].map((item) => [item.record_id, item]));

describe('inherited values (data-model 8.2-8.3)', () => {
  it('builds the chain from the nearest group to the top', () => {
    expect(inheritanceChain('g-edge', byId).map((item) => item.data.name)).toEqual(['edge', 'web', 'prod']);
    expect(inheritanceChain(null, byId)).toEqual([]);
    expect(inheritanceChain('missing', byId)).toEqual([]);
  });

  it('takes each value from the nearest group that sets it', () => {
    const inherited = resolveInherited('g-edge', byId);
    expect(inherited.username?.value).toBe('www');
    expect(inherited.username?.group.data.name).toBe('web');
    expect(inherited.port?.value).toBe(2222);
    expect(inherited.port?.group.data.name).toBe('prod');
  });

  it('inherits nothing at the top level or from groups without values', () => {
    expect(resolveInherited(null, byId)).toEqual({});
    expect(resolveInherited('g-lone', byId)).toEqual({});
  });

  it('stops at a missing ancestor', () => {
    const orphan = group('g-orphan', 'orphan', ['g-gone', 'g-web']);
    const map = new Map(byId).set('g-orphan', orphan);
    // Chain: orphan, web, then the missing top-level group ends it.
    expect(resolveInherited('g-orphan', map).username?.value).toBe('www');
    expect(resolveInherited('g-orphan', map).port).toBeUndefined();
    const cut = group('g-cut', 'cut', ['g-prod', 'g-gone']);
    expect(resolveInherited('g-cut', new Map(byId).set('g-cut', cut))).toEqual({});
  });

  it('formats the "from group X" placeholder', () => {
    const inherited = resolveInherited('g-edge', byId);
    expect(inheritedPlaceholder(inherited.port, '22')).toBe('2222 (from group prod)');
    expect(inheritedPlaceholder(inherited.username)).toBe('www (from group web)');
    expect(inheritedPlaceholder(undefined, '22')).toBe('22');
  });
});
