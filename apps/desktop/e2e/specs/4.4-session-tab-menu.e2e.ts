// UI 4.4 Session tab context menu: Duplicate (new tab, same host),
// Duplicate in a new window, Rename (the host keeps its name), Split
// horizontally (new pane below), Split vertically (new pane to the right),
// Close. New panes start with the host picker (UI 4.7). Start multiplayer
// (stage 10) and Broadcast input (stage 9) are hidden.

import { Key } from 'webdriverio';
import {
  activateVaults,
  cardText,
  clearAndType,
  closeWindowConfirmed,
  createSshHost,
  el,
  expectAbsent,
  focusTerminal,
  inPane,
  menuItemIds,
  openSshSession,
  openTabMenu,
  paneInfos,
  pressEscape,
  startFresh,
  tabIds,
  tabTitle,
  waitFor,
  waitForAbsent,
  waitForBoot,
  waitForLockState,
  waitForNewTab,
  waitForNewWindow,
  waitForSelectedTab,
  waitForShell,
  waitForTabClosed,
  waitForTabCount,
  type PaneInfo,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

const MENU = [
  'session-tab-menu-duplicate',
  'session-tab-menu-duplicate-window',
  'session-tab-menu-rename',
  'session-tab-menu-split-horizontal',
  'session-tab-menu-split-vertical',
  'session-tab-menu-close',
];

const HIDDEN = [
  'session-tab-menu-multiplayer', // stage 10
  'session-tab-menu-broadcast', // stage 9
];

async function waitForPaneCount(n: number): Promise<PaneInfo[]> {
  let panes: PaneInfo[] = [];
  await browser.waitUntil(
    async () => {
      panes = await paneInfos();
      return panes.length === n;
    },
    { timeout: 10_000, timeoutMsg: `expected ${n} panes in the active tab` },
  );
  return panes;
}

describe('UI 4.4 Session tab context menu', () => {
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;
  let first: string;
  let duplicate: string;

  before(async () => {
    await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'Menu host' });
    first = await openSshSession(hostId, user.name);
  });

  after(async () => {
    await stopAllSshd();
  });

  it('lists the stage-2 items in order', async () => {
    await openTabMenu(first);
    expect(await menuItemIds('session-tab-menu')).toEqual(MENU);
    await expectAbsent(HIDDEN);
    await pressEscape();
    await waitForAbsent('session-tab-menu');
  });

  it('duplicates the session into a new tab to the same host', async () => {
    const before = await tabIds();
    const [original] = await paneInfos(first);
    await openTabMenu(first);
    await el('session-tab-menu-duplicate').click();
    duplicate = await waitForNewTab(before);
    await waitForSelectedTab(duplicate);
    await waitForShell(user.name);
    expect(await tabTitle(duplicate)).toBe('Menu host');
    const [copy] = await paneInfos(duplicate);
    expect(copy.sessionId).not.toBeNull();
    expect(copy.sessionId).not.toBe(original.sessionId);
    expect(await tabIds()).toEqual([first, duplicate]);
  });

  it('duplicates the session into a new window', async () => {
    const main = await browser.getWindowHandle();
    const handles = await browser.getWindowHandles();
    await openTabMenu(first);
    await el('session-tab-menu-duplicate-window').click();
    const second = await waitForNewWindow(handles);
    await browser.switchToWindow(second);
    await waitForBoot();
    await expect(el('app-root')).toHaveAttribute('data-window-label', 'w-2');
    await waitForLockState('unlocked');
    const [tab] = await waitForTabCount(1);
    await waitForSelectedTab(tab);
    await waitForShell(user.name);
    expect(await tabTitle(tab)).toBe('Menu host');
    // The window owns an active session: closing it asks first.
    await closeWindowConfirmed(main);
    await waitForTabCount(2);
  });

  it('renames the tab without renaming the host', async () => {
    await openTabMenu(first);
    await el('session-tab-menu-rename').click();
    await waitFor('titlebar-session-tab-rename-input');
    await clearAndType('titlebar-session-tab-rename-input', 'Renamed tab');
    await browser.keys(Key.Enter);
    await waitForAbsent('titlebar-session-tab-rename-input');
    await browser.waitUntil(async () => (await tabTitle(first)) === 'Renamed tab', {
      timeout: 5_000,
      timeoutMsg: 'the tab title did not change',
    });

    // Escape cancels a rename.
    await openTabMenu(first);
    await el('session-tab-menu-rename').click();
    await waitFor('titlebar-session-tab-rename-input');
    await clearAndType('titlebar-session-tab-rename-input', 'Discarded');
    await pressEscape();
    await waitForAbsent('titlebar-session-tab-rename-input');
    expect(await tabTitle(first)).toBe('Renamed tab');

    await activateVaults();
    expect(await cardText('host-card', hostId, 'host-card-title')).toBe('Menu host');
  });

  it('splits horizontally: a new pane below with the host picker', async () => {
    await el('titlebar-session-tab', `[data-tab-id="${first}"]`).click();
    await waitForSelectedTab(first);
    const [original] = await waitForPaneCount(1);
    await openTabMenu(first);
    await el('session-tab-menu-split-horizontal').click();
    const panes = await waitForPaneCount(2);
    const fresh = panes.find((pane) => pane.id !== original.id)!;
    const top = panes.find((pane) => pane.id === original.id)!;
    expect(fresh.sessionId).toBeNull();
    expect(fresh.rect.y).toBeGreaterThanOrEqual(top.rect.y + top.rect.height - 2);
    await $(inPane(fresh.id, 'pane-host-picker-search-input')).waitForExist();
    expect(await browser.execute((selector: string) => document.querySelectorAll(selector).length, '[data-testid="split-divider"]')).toBeGreaterThanOrEqual(1);
  });

  it('splits vertically: a new pane to the right with the host picker', async () => {
    const before = await paneInfos();
    const original = before.find((pane) => pane.sessionId !== null)!;
    await focusTerminal(original.id);
    await openTabMenu(first);
    await el('session-tab-menu-split-vertical').click();
    const panes = await waitForPaneCount(3);
    const fresh = panes.find((pane) => !before.some((old) => old.id === pane.id))!;
    const left = panes.find((pane) => pane.id === original.id)!;
    expect(fresh.sessionId).toBeNull();
    expect(fresh.rect.x).toBeGreaterThanOrEqual(left.rect.x + left.rect.width - 2);
    await $(inPane(fresh.id, 'pane-host-picker-search-input')).waitForExist();

    // A tab with splits still has no Broadcast input or multiplayer items.
    await openTabMenu(first);
    await expectAbsent(HIDDEN);
    await pressEscape();
    await waitForAbsent('session-tab-menu');
  });

  it('closes the tab and its sessions with Close', async () => {
    await openTabMenu(first);
    await el('session-tab-menu-close').click();
    await waitForTabClosed(first);
    // No session tab on the left: the right neighbor becomes active.
    await waitForSelectedTab(duplicate);
    expect(await tabIds()).toEqual([duplicate]);
  });
});
