// UI 6 New tab page: "+" opens a "New Tab" with the search focused and the
// Ctrl+K hint; Recent connections (OS icon, name, "vault / group") connect
// in the same tab by click or arrows and Enter; typing lists open tabs first,
// then hosts; Ctrl+K anywhere and Ctrl+Shift+K in the terminal open the same
// search as a palette (Ctrl+K in the terminal goes to the shell); Restore is
// disabled during the first launch (also after tabs were opened) and reopens
// the tabs of the previous launch once, also after a new session was opened.
// "Create a workspace" and the workspaces block are stage 9 (hidden).

import { Key } from 'webdriverio';
import {
  activateTab,
  activateVaults,
  closeTab,
  createGroup,
  createSshHost,
  el,
  expectAbsent,
  focusTerminal,
  inNewTab,
  openFolder,
  openNewTabPage,
  openSshSession,
  pickerResults,
  pressCtrl,
  pressCtrlShift,
  pressEscape,
  restartAndUnlock,
  runCommand,
  sessionTab,
  sessionTabs,
  startFresh,
  tabIds,
  tabTitle,
  textAt,
  tid,
  waitFor,
  waitForAbsent,
  waitForFocus,
  waitForFocusAt,
  waitForNewTab,
  waitForSelectedTab,
  waitForShell,
  waitForTabClosed,
  type Session,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

const HIDDEN = [
  'newtab-create-workspace', // stage 9
  'newtab-workspaces', // stage 9
];

describe('UI 6 New tab page', () => {
  let session: Session;
  let sshd: Sshd;
  let user: SshdUser;
  let groupId: string;
  let hostId: string;
  let newTab: string;
  let sessionTabId: string;

  before(async () => {
    session = await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    groupId = await createGroup({ name: 'Servers' });
    hostId = await createSshHost(sshd, user, { label: 'Recent host', groupId });
  });

  after(async () => {
    await stopAllSshd();
  });

  it('opens "New Tab" with the focused search and the Ctrl+K hint', async () => {
    newTab = await openNewTabPage();
    await expect(sessionTab(newTab)).toHaveAttribute('data-kind', 'newtab');
    expect(await tabTitle(newTab)).toBe('New Tab');
    await waitForFocusAt(inNewTab(newTab, 'newtab-search-input'));
    expect(await textAt(inNewTab(newTab, 'newtab-search-hint'))).toContain('Ctrl+K');
    await $(inNewTab(newTab, 'newtab-recent-empty')).waitForExist();
  });

  it('disables Restore at the first launch and hides the workspace elements', async () => {
    await expect($(inNewTab(newTab, 'newtab-restore'))).toBeDisabled();
    await expectAbsent(HIDDEN);
  });

  it('lists a host in Recent connections after a connection', async () => {
    await closeTab(newTab);
    await waitForSelectedTab('vaults');
    await openFolder(groupId);
    const tab = await openSshSession(hostId, user.name);
    await runCommand('exit');
    await waitForTabClosed(tab);

    newTab = await openNewTabPage();
    const item = inNewTab(newTab, 'newtab-recent-item', `[data-host-id="${hostId}"]`);
    await $(item).waitForExist({ timeout: 10_000 });
    expect(await textAt(`${item} ${tid('newtab-recent-item-title')}`)).toBe('Recent host');
    expect(await textAt(`${item} ${tid('newtab-recent-item-path')}`)).toBe('Personal / Servers');
    await expect($(`${item} ${tid('newtab-recent-item-os-icon')}`)).toBeExisting();
  });

  it('connects in the same tab on a click', async () => {
    await $(inNewTab(newTab, 'newtab-recent-item', `[data-host-id="${hostId}"]`)).click();
    await waitForShell(user.name);
    await expect(sessionTab(newTab)).toHaveAttribute('data-kind', 'session');
    expect(await tabTitle(newTab)).toBe('Recent host');
    expect(await tabIds()).toEqual([newTab]);
    await runCommand('exit');
    await waitForTabClosed(newTab);
  });

  it('connects with the arrow keys and Enter', async () => {
    newTab = await openNewTabPage();
    const item = inNewTab(newTab, 'newtab-recent-item', `[data-host-id="${hostId}"]`);
    await $(item).waitForExist({ timeout: 10_000 });
    await waitForFocusAt(inNewTab(newTab, 'newtab-search-input'));
    await browser.keys(Key.ArrowDown);
    await browser.waitUntil(async () => (await $(item).getAttribute('aria-selected')) === 'true', {
      timeout: 5_000,
      timeoutMsg: 'ArrowDown did not move the cursor to the recent connection',
    });
    await browser.keys(Key.Enter);
    await waitForShell(user.name);
    await expect(sessionTab(newTab)).toHaveAttribute('data-kind', 'session');
    sessionTabId = newTab;
  });

  it('lists open tabs first, then hosts, while typing', async () => {
    newTab = await openNewTabPage();
    const search = inNewTab(newTab, 'newtab-search-input');
    await waitForFocusAt(search);
    await $(search).addValue('Recent');
    await $(inNewTab(newTab, 'newtab-results')).waitForExist();
    const scope = tid('newtab-page', `[data-tab-id="${newTab}"]`);
    await browser.waitUntil(async () => (await pickerResults(scope, 'newtab')).length >= 2, {
      timeout: 5_000,
      timeoutMsg: 'the results do not list the open tab and the host',
    });
    const results = await pickerResults(scope, 'newtab');
    const tabIndex = results.findIndex((result) => result.kind === 'tab' && result.id === sessionTabId);
    const hostIndex = results.findIndex((result) => result.kind === 'host' && result.id === hostId);
    expect(tabIndex).toBe(0);
    expect(hostIndex).toBeGreaterThan(tabIndex);

    // Choosing an open tab activates it; the new tab page stays.
    await $(inNewTab(newTab, 'newtab-result-tab', `[data-tab-id="${sessionTabId}"]`)).click();
    await waitForSelectedTab(sessionTabId);
    expect(await tabIds()).toContain(newTab);
  });

  it('opens the search palette with Ctrl+K on the Hosts screen', async () => {
    await activateVaults();
    await pressCtrl('k');
    await waitFor('search-palette');
    await waitForFocus('search-palette-search-input');
    await pressEscape();
    await waitForAbsent('search-palette');
  });

  it('leaves Ctrl+K to the shell in the terminal and opens the palette with Ctrl+Shift+K', async () => {
    await activateTab(sessionTabId);
    await focusTerminal();
    await pressCtrl('k');
    await browser.pause(700);
    await expectAbsent(['search-palette']);
    await pressCtrlShift('k');
    await waitFor('search-palette');
    await pressEscape();
    await waitForAbsent('search-palette');
  });

  it('connects a host chosen in the search palette in a new tab', async () => {
    const before = await tabIds();
    await pressCtrlShift('k');
    await waitFor('search-palette');
    await waitForFocus('search-palette-search-input');
    await el('search-palette-search-input').addValue('Recent');
    await (await waitFor('search-palette-result-host', `[data-host-id="${hostId}"]`)).click();
    await waitForAbsent('search-palette');
    const tab = await waitForNewTab(before);
    await waitForSelectedTab(tab);
    await waitForShell(user.name);
  });

  it('keeps Restore disabled during the first launch after tabs were opened', async () => {
    const page = await openNewTabPage();
    // session.last is written 2 s after the last change: for the next launch only.
    await browser.pause(3_000);
    await expect($(inNewTab(page, 'newtab-restore'))).toBeDisabled();
    await closeTab(page);
    await waitForTabClosed(page);
  });

  it('restores the tabs of the previous launch once, also after a new session', async () => {
    const saved = (await sessionTabs()).filter((tab) => tab.kind === 'session').length;
    expect(saved).toBe(2);
    await restartAndUnlock(session.password);
    expect(await tabIds()).toEqual([]);

    // A new session first: it rewrites session.last for the next launch.
    newTab = await openNewTabPage();
    const item = inNewTab(newTab, 'newtab-recent-item', `[data-host-id="${hostId}"]`);
    await $(item).waitForExist({ timeout: 10_000 });
    await $(item).click();
    await waitForShell(user.name);
    await browser.pause(3_000);

    const page = await openNewTabPage();
    const restore = $(inNewTab(page, 'newtab-restore'));
    await restore.waitForEnabled({ timeout: 10_000 });
    await restore.click();
    await browser.waitUntil(async () => (await sessionTabs()).filter((tab) => tab.kind === 'session').length >= saved + 1, {
      timeout: 20_000,
      timeoutMsg: `Restore did not reopen ${saved} session tabs`,
    });
    const restored = (await sessionTabs()).filter((tab) => tab.kind === 'session');
    expect(restored).toHaveLength(saved + 1);
    for (const tab of restored) {
      expect(tab.title).toBe('Recent host');
    }
    // Used once: Restore is disabled afterwards.
    await expect($(inNewTab(page, 'newtab-restore'))).toBeDisabled();
    // The restored sessions connect (the host key is known).
    await activateTab(restored[restored.length - 1].id);
    await waitForShell(user.name);
  });
});
