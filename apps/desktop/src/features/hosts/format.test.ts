import { describe, expect, it } from 'vitest';
import type { GroupView, HostData, HostView } from '../../ipc';
import {
  buildListQuery,
  collectTags,
  groupPathLabel,
  hostSubtitle,
  hostTitle,
  isFiltering,
  sortedByPath,
  subtreeIds,
} from './format';

const VAULT = 'v-1';

function host(data: HostData, extra: Partial<HostView> = {}): HostView {
  return {
    type: 'host',
    vault_id: VAULT,
    record_id: `h-${data.address}`,
    revision: 1,
    created_at: '2026-10-18T00:00:00.000Z',
    modified_at: '2026-10-18T00:00:00.000Z',
    inherited: {},
    path: [],
    secrets_set: [],
    data,
    ...extra,
  };
}

function group(id: string, name: string, path: string[] = []): GroupView {
  return {
    type: 'group',
    vault_id: VAULT,
    record_id: id,
    revision: 1,
    created_at: '2026-10-18T00:00:00.000Z',
    modified_at: '2026-10-18T00:00:00.000Z',
    inherited: {},
    path,
    data: { name },
    host_count: 0,
    child_count: 0,
  };
}

describe('list query (stage plan 5.4, 6.6)', () => {
  it('shows the open folder without a filter', () => {
    expect(buildListQuery({ vaultId: VAULT, folderId: null, text: '', tags: [], sort: 'manual' })).toEqual({
      vault_id: VAULT,
      types: ['group', 'host'],
      scope: { kind: 'top_level' },
      sort: 'manual',
    });
    expect(buildListQuery({ vaultId: VAULT, folderId: 'g-1', text: '  ', tags: [], sort: 'name' })).toEqual({
      vault_id: VAULT,
      types: ['group', 'host'],
      scope: { kind: 'group', group_id: 'g-1' },
      sort: 'name',
    });
  });

  it('searches the whole vault while text or tags are set', () => {
    expect(buildListQuery({ vaultId: VAULT, folderId: 'g-1', text: ' db ', tags: ['prod'], sort: 'created' })).toEqual({
      vault_id: VAULT,
      types: ['group', 'host'],
      scope: { kind: 'all' },
      text: 'db',
      tags: ['prod'],
      sort: 'created',
    });
    expect(isFiltering('', [])).toBe(false);
    expect(isFiltering('', ['a'])).toBe(true);
  });
});

describe('card text (UI 2.3)', () => {
  const groups = new Map([['g-1', group('g-1', 'prod')]]);

  it('uses the label or the address as the title', () => {
    expect(hostTitle(host({ address: 'db1.example.com', label: 'DB' }))).toBe('DB');
    expect(hostTitle(host({ address: 'db1.example.com', label: '  ' }))).toBe('db1.example.com');
  });

  it('builds the subtitle from protocol, username, then tags or the group', () => {
    const own = host({ address: 'a.example.com', group_id: 'g-1', tags: ['db', 'eu'], ssh: { username: 'ops' } });
    expect(hostSubtitle(own, false, groups)).toBe('SSH, ops, db, eu');
    expect(hostSubtitle(own, true, groups)).toBe('SSH, ops, prod');
    const inherited = host(
      { address: 'b.example.com' },
      { inherited: { username: { value: 'deploy', from_group_id: 'g-1' } } },
    );
    expect(hostSubtitle(inherited, false, groups)).toBe('SSH, deploy');
    expect(hostSubtitle(host({ address: 'c.example.com' }), true, groups)).toBe('SSH');
  });

  it('collects distinct tags case-insensitively', () => {
    expect(collectTags([host({ address: 'a', tags: ['Prod', 'db'] }), host({ address: 'b', tags: ['prod', 'api'] })])).toEqual([
      'api',
      'db',
      'Prod',
    ]);
  });

  it('finds a group subtree and labels groups with their path', () => {
    const all = [group('a', 'prod'), group('b', 'web', ['a']), group('c', 'edge', ['a', 'b']), group('d', 'other')];
    expect([...subtreeIds('b', all)].sort()).toEqual(['b', 'c']);
    const byId = new Map(all.map((item) => [item.record_id, item]));
    expect(groupPathLabel(all[2] as GroupView, byId)).toBe('prod / web / edge');
    expect(sortedByPath(all, byId).map((entry) => entry.label)).toEqual(['other', 'prod', 'prod / web', 'prod / web / edge']);
  });
});
