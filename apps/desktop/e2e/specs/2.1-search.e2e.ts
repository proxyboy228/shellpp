// UI 2.1 Search: real-time filter of groups and hosts by name, address,
// tags and username (own or inherited) across the whole vault; clearing
// returns to the current folder. Quick connect is stage 2 (hidden).

import {
  clearSearch,
  count,
  createGroup,
  createHost,
  el,
  expectAbsent,
  groupIds,
  search,
  startFresh,
  waitFor,
  waitForGroupIds,
  waitForHostIds,
} from '../helpers/index.ts';

describe('UI 2.1 Search', () => {
  let prod: string;
  let db: string;
  let primaryDb: string;
  let webEdge: string;
  let appHost: string;

  before(async () => {
    await startFresh();
    prod = await createGroup({ name: 'prod', username: 'deploy' });
    db = await createGroup({ name: 'db', parentId: prod });
    primaryDb = await createHost({ address: 'db1.example.com', label: 'Primary DB', tags: ['postgres'], groupId: db });
    webEdge = await createHost({ address: '192.0.2.10', label: 'Web edge', username: 'alice', tags: ['nginx', 'edge'] });
    appHost = await createHost({ address: 'app.example.org', groupId: prod });
    await waitForGroupIds([prod]);
    await waitForHostIds([webEdge]);
  });

  it('shows a full-width search bar with a placeholder', async () => {
    const placeholder = (await el('hosts-search-input').getAttribute('placeholder')) ?? '';
    expect(placeholder).not.toBe('');
    // The quick-connect hint belongs to quick connect (stage 2, hidden).
    if ((await count('hosts-search-connect')) === 0) {
      expect(placeholder).not.toContain('ssh user@hostname');
    } else {
      expect(placeholder).toContain('ssh user@hostname');
    }
  });

  it('filters hosts by label, across nested groups', async () => {
    await search('primary');
    await waitForHostIds([primaryDb]);
    expect(await count('group-card')).toBe(0);
  });

  it('filters hosts by address', async () => {
    await search('192.0.2');
    await waitForHostIds([webEdge]);
  });

  it('filters hosts by tag', async () => {
    await search('postgres');
    await waitForHostIds([primaryDb]);
    await search('EDGE');
    await waitForHostIds([webEdge]);
  });

  it('filters hosts by their own username', async () => {
    await search('alice');
    await waitForHostIds([webEdge]);
  });

  it('filters hosts by the username inherited from a group', async () => {
    await search('deploy');
    await waitForHostIds([primaryDb, appHost]);
    expect(await count('group-card')).toBe(0);
  });

  it('filters groups by name', async () => {
    await search('db');
    await waitForGroupIds([db]);
    // "db1.example.com" matches by address too.
    await waitForHostIds([primaryDb]);
  });

  it('shows "no results" when nothing matches', async () => {
    await search('nothing-matches-this');
    await waitFor('hosts-no-results');
    expect(await count('host-card')).toBe(0);
    expect(await groupIds()).toEqual([]);
  });

  it('does not render the quick connect button (stage 2)', async () => {
    await search('user@192.0.2.10');
    await expectAbsent(['hosts-search-connect']);
  });

  it('returns to the current folder when the search is cleared', async () => {
    await search('alice');
    await waitForHostIds([webEdge]);
    await clearSearch();
    await waitForGroupIds([prod]);
    await waitForHostIds([webEdge]);
    await expectAbsent(['hosts-no-results']);
    await expect(el('hosts-search-clear')).not.toBeDisplayed();
  });
});
