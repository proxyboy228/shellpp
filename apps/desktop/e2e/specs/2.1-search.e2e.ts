// UI 2.1 Search and quick connect: real-time filter of groups and hosts by
// name, address, tags and username (own or inherited) across the whole vault;
// clearing returns to the current folder. Stage 2: the Connect button is
// enabled for a connection address or an exact label/address match; Enter
// quick connects without saving (a missing username or password is asked,
// UI 4.8), an exact match connects to the saved host with its settings, and a
// successful quick connect offers to save the host.

import {
  activateVaults,
  answerPrompt,
  clearSearch,
  count,
  createGroup,
  createHost,
  createSshHost,
  el,
  expectAbsent,
  groupIds,
  hostIds,
  hostKey,
  quickConnect,
  quickTarget,
  runCommand,
  search,
  startFresh,
  tabIds,
  tabTitle,
  valueOf,
  waitFor,
  waitForGroupIds,
  waitForHostIds,
  waitForNewTab,
  waitForSelectedTab,
  waitForShell,
  waitForTabClosed,
  waitForToast,
  waitInPane,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

/** Types the text and returns whether Connect is enabled. */
async function connectEnabled(text: string): Promise<boolean> {
  await search(text);
  const button = el('hosts-search-connect');
  await button.waitForExist();
  // Give the (debounced) search a moment; the button follows the text.
  await browser.pause(200);
  return button.isEnabled();
}

describe('UI 2.1 Search and quick connect', () => {
  let sshd: Sshd;
  let user: SshdUser;
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
    sshd = await startSshd();
    user = sshd.users[0];
  });

  after(async () => {
    await stopAllSshd();
  });

  it('shows a full-width search bar with the quick connect hint and a Connect button', async () => {
    const placeholder = (await el('hosts-search-input').getAttribute('placeholder')) ?? '';
    expect(placeholder).toContain('ssh user@hostname');
    await expect(el('hosts-search-connect')).toBeDisplayed();
    await expect(el('hosts-search-connect')).toBeDisabled();
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

  it('returns to the current folder when the search is cleared', async () => {
    await search('alice');
    await waitForHostIds([webEdge]);
    await clearSearch();
    await waitForGroupIds([prod]);
    await waitForHostIds([webEdge]);
    await expectAbsent(['hosts-no-results']);
    await expect(el('hosts-search-clear')).not.toBeDisplayed();
  });

  it('enables Connect only for a connection address or an exact match', async () => {
    // Plain words are search queries only.
    expect(await connectEnabled('prod')).toBe(false);
    expect(await connectEnabled('nothing-matches-this')).toBe(false);
    expect(await connectEnabled('Web')).toBe(false);
    // Connection addresses (nothing is connected here).
    expect(await connectEnabled('user@192.0.2.10')).toBe(true);
    expect(await connectEnabled('ops@192.0.2.10:2222')).toBe(true);
    expect(await connectEnabled('ssh alice@db1.example.com -p 2222')).toBe(true);
    expect(await connectEnabled('db1.example.com')).toBe(true);
    expect(await connectEnabled('198.51.100.7')).toBe(true);
    expect(await connectEnabled('localhost')).toBe(true);
    // An exact label match, case-insensitive.
    expect(await connectEnabled('Web edge')).toBe(true);
    expect(await connectEnabled('web EDGE')).toBe(true);
    await clearSearch();
  });

  it('quick connects with Enter without saving the host, asking for the password', async () => {
    const tab = await quickConnect(quickTarget(sshd, user.name));
    expect(tab).not.toBeNull();
    await hostKey('accept');
    await waitInPane('session-prompt', '[data-kind="password"]');
    // No "Save" for a quick connect without a saved host (UI 4.8).
    await expectAbsent(['session-prompt-save']);
    await answerPrompt(user.password as string, { kind: 'password' });
    await waitForShell(user.name);
  });

  it('offers to save the host after the quick connect', async () => {
    await waitForToast('hosts-quick-connect-save-toast');
    await el('hosts-quick-connect-save-toast-action').click();
    await activateVaults();
    let saved = '';
    await browser.waitUntil(
      async () => {
        saved = await browser.execute(
          () => document.querySelector('[data-testid="host-panel"]')?.getAttribute('data-record-id') ?? '',
        );
        return saved !== '' && saved !== webEdge;
      },
      { timeout: 10_000, timeoutMsg: 'Save host did not create the host and open its panel' },
    );
    expect(await valueOf('host-panel-address')).toBe(sshd.address);
    expect(await valueOf('host-panel-ssh-port')).toBe(String(sshd.port));
    expect(await valueOf('host-panel-ssh-username')).toBe(user.name);
    await el('host-panel-collapse').click();
    // The quick connect itself saved nothing: Save host added the only new host.
    await search('');
    await waitForHostIds([webEdge, saved]);
    expect(await hostIds()).toHaveLength(2);

    // End the quick-connect session.
    const [tab] = await tabIds();
    await el('titlebar-session-tab', `[data-tab-id="${tab}"]`).click();
    await waitForSelectedTab(tab);
    await runCommand('exit');
    await waitForTabClosed(tab);
  });

  it('connects to the saved host with its settings on an exact label match', async () => {
    await createSshHost(sshd, user, { label: 'Exact host' });
    await activateVaults();
    await search('Exact host');
    const button = el('hosts-search-connect');
    await button.waitForEnabled({ timeout: 5_000 });
    const before = await tabIds();
    await button.click();
    const tab = await waitForNewTab(before);
    await waitForSelectedTab(tab);
    // The saved password and the saved host key: no question.
    await waitForShell(user.name);
    expect(await tabTitle(tab)).toBe('Exact host');
    // No save offer for a saved host.
    await browser.pause(1_500);
    await expectAbsent(['hosts-quick-connect-save-toast']);
    await runCommand('exit');
    await waitForTabClosed(tab);
    await activateVaults();
    await search('');
  });
});
