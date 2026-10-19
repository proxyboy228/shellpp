// UI 1.1 Top bar: custom frameless title bar with the app menu button, the
// persistent Vaults tab, the session tabs and "+" (stage 2), the
// notification center button, the terminal sidebar toggle (only while a
// session tab is active) and the window buttons. While locked the session
// tabs are not shown. Elements of later stages are not rendered.

import {
  activateTab,
  activateVaults,
  appMenu,
  appOs,
  el,
  expectAbsent,
  expectPresent,
  openNewTabPage,
  pressCtrlShift,
  runCommand,
  sessionTab,
  startFresh,
  tabIds,
  unlock,
  waitFor,
  waitForLocalShell,
  waitForLockState,
  waitForNewTab,
  waitForPaneState,
  waitForSelectedTab,
  waitForTabClosed,
  type Session,
} from '../helpers/index.ts';

const HIDDEN = [
  'titlebar-vault-switcher', // stage 8
  'titlebar-tab-sftp', // stage 4
  'titlebar-update-button', // stage 9
];

async function maximized(): Promise<string | null> {
  return el('titlebar-window-maximize').getAttribute('data-maximized');
}

describe('UI 1.1 Top bar', () => {
  let os: string | null;
  let session: Session;
  let localTab: string;

  before(async () => {
    session = await startFresh();
    os = await appOs();
  });

  it('shows the custom title bar with the Vaults tab selected', async () => {
    await expect(el('titlebar')).toBeDisplayed();
    await expect(el('titlebar-tab-vaults')).toBeDisplayed();
    await expect(el('titlebar-tab-vaults')).toHaveAttribute('aria-selected', 'true');
  });

  it('shows the notification center button without a counter when nothing is unread', async () => {
    await expect(el('titlebar-notifications-button')).toBeDisplayed();
    await expectAbsent(['titlebar-notifications-badge']);
  });

  it('shows the app menu button and the window buttons on Windows and Linux', async function () {
    if (os === 'macos') {
      // macOS uses the native menu bar and the traffic lights (manual checklist).
      await expectAbsent(['titlebar-menu-button', 'titlebar-window-minimize', 'titlebar-window-maximize', 'titlebar-window-close']);
      this.skip();
    }
    await expectPresent(['titlebar-menu-button', 'titlebar-window-minimize', 'titlebar-window-maximize', 'titlebar-window-close']);
    // Window buttons sit on the right of the bar, the menu button on the left.
    const bar = await el('titlebar').getLocation();
    const barSize = await el('titlebar').getSize();
    const menu = await el('titlebar-menu-button').getLocation();
    const close = await el('titlebar-window-close').getLocation();
    expect(menu.x - bar.x).toBeLessThan(barSize.width / 2);
    expect(close.x - bar.x).toBeGreaterThan(barSize.width / 2);
  });

  it('toggles maximize and restore with the maximize button', async function () {
    if (os === 'macos') {
      this.skip();
    }
    const before = await maximized();
    await el('titlebar-window-maximize').click();
    await browser.waitUntil(async () => (await maximized()) !== before, {
      timeout: 5_000,
      timeoutMsg: 'data-maximized did not change after maximize',
    });
    await el('titlebar-window-maximize').click();
    await browser.waitUntil(async () => (await maximized()) === before, {
      timeout: 5_000,
      timeoutMsg: 'data-maximized did not change back after restore',
    });
  });

  it('toggles maximize with a double click on the empty title bar area', async function () {
    if (os === 'macos') {
      this.skip();
    }
    const before = await maximized();
    // The last element of the tab strip is "+" (stage 2), else the Vaults tab.
    const lastTab = (await el('titlebar-new-tab').isExisting()) ? el('titlebar-new-tab') : el('titlebar-tab-vaults');
    const tab = await lastTab.getLocation();
    const tabSize = await lastTab.getSize();
    const vaults = await el('titlebar-tab-vaults').getLocation();
    const vaultsSize = await el('titlebar-tab-vaults').getSize();
    const right = await el('titlebar-notifications-button').getLocation();
    // A point between the tab strip and the right-hand buttons.
    const x = Math.round((tab.x + tabSize.width + right.x) / 2);
    const y = Math.round(vaults.y + vaultsSize.height / 2);
    const doubleClickAt = async (): Promise<void> => {
      await browser
        .action('pointer', { parameters: { pointerType: 'mouse' } })
        .move({ origin: 'viewport', x, y })
        .down()
        .up()
        .pause(30)
        .down()
        .up()
        .perform();
    };
    await doubleClickAt();
    await browser.waitUntil(async () => (await maximized()) !== before, {
      timeout: 5_000,
      timeoutMsg: 'double click on the title bar did not toggle maximize',
    });
    await doubleClickAt();
    await browser.waitUntil(async () => (await maximized()) === before, {
      timeout: 5_000,
      timeoutMsg: 'second double click did not restore the window',
    });
  });

  it('shows "+" and no session tabs or terminal sidebar toggle on the Vaults tab', async () => {
    await expect(el('titlebar-new-tab')).toBeDisplayed();
    expect(await tabIds()).toEqual([]);
    await expectAbsent(['titlebar-terminal-sidebar-toggle']);
  });

  it('opens a new tab with "+"; the sidebar toggle stays hidden on it', async () => {
    const tab = await openNewTabPage();
    await expect(sessionTab(tab)).toHaveAttribute('aria-selected', 'true');
    await expect(el('titlebar-tab-vaults')).toHaveAttribute('aria-selected', 'false');
    await expectAbsent(['titlebar-terminal-sidebar-toggle']);
    // On the active tab the close button replaces the OS icon.
    await expectPresent(['titlebar-session-tab-close']);
    await el('titlebar-session-tab-close').click();
    await waitForTabClosed(tab);
    await waitForSelectedTab('vaults');
  });

  it('shows a session tab with the close button, the type icon and the sidebar toggle', async function () {
    if (os === 'macos') {
      this.skip();
    }
    const before = await tabIds();
    await appMenu('app-menu-new-local-terminal');
    localTab = await waitForNewTab(before);
    await waitForSelectedTab(localTab);
    await waitForPaneState('connected', { timeout: 30_000 });
    await waitForLocalShell();
    await expect(sessionTab(localTab)).toHaveAttribute('data-kind', 'session');
    await expect(sessionTab(localTab)).toHaveAttribute('data-state', 'connected');
    await expect($(`[data-testid="titlebar-session-tab"][data-tab-id="${localTab}"] [data-testid="titlebar-session-tab-kind"]`)).toHaveAttribute(
      'data-kind',
      'terminal',
    );
    await expect($(`[data-testid="titlebar-session-tab"][data-tab-id="${localTab}"] [data-testid="titlebar-session-tab-close"]`)).toBeExisting();
    await expect(el('titlebar-terminal-sidebar-toggle')).toBeDisplayed();
    // Back on Vaults: the tab stays, the toggle goes; the inactive tab shows its OS icon.
    await activateVaults();
    await expectAbsent(['titlebar-terminal-sidebar-toggle']);
    await expect(
      $(`[data-testid="titlebar-session-tab"][data-tab-id="${localTab}"] [data-testid="titlebar-session-tab-os-icon"]`),
    ).toBeExisting();
    await activateTab(localTab);
    await waitFor('titlebar-terminal-sidebar-toggle');
  });

  it('does not show the session tabs while locked', async function () {
    if (os === 'macos') {
      this.skip();
    }
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await expectAbsent(['titlebar-session-tab', 'titlebar-new-tab', 'titlebar-terminal-sidebar-toggle']);
    await unlock(session.password);
    await sessionTab(localTab).waitForExist({ timeout: 10_000 });
  });

  it('closes the session tab with its close button', async function () {
    if (os === 'macos') {
      this.skip();
    }
    await activateTab(localTab);
    await $(`[data-testid="titlebar-session-tab"][data-tab-id="${localTab}"] [data-testid="titlebar-session-tab-close"]`).click();
    await waitForTabClosed(localTab);
    await waitForSelectedTab('vaults');
    // Ctrl+Shift+L still opens a local terminal from the Vaults tab (UI 12.5).
    const before = await tabIds();
    await pressCtrlShift('l');
    const tab = await waitForNewTab(before);
    await waitForPaneState('connected', { timeout: 30_000 });
    await waitForLocalShell();
    await runCommand('exit');
    await waitForTabClosed(tab, 20_000);
  });

  it('does not render elements of later stages', async () => {
    await expectAbsent(HIDDEN);
  });
});
