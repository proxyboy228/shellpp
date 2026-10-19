// UI 1.4 App menu: items in order with symbol shortcut hints, Settings
// (menu and Ctrl+,), New Local Terminal (menu and Ctrl+Shift+L, stage 2),
// Edit, View zoom and sidebar, Window > New window (all windows are equal:
// data and settings changes appear in every window at once) and the tab
// items (next, previous, close; stage 2), Help links (through the mock
// launcher) and About, Lock, Exit. SFTP, New Serial Connection and Check for
// updates belong to later stages (hidden).

import { Key } from 'webdriverio';
import {
  appMenu,
  appOs,
  appVersion,
  cardText,
  clipboardText,
  closeAppMenu,
  closePanel,
  closeSettings,
  createHost,
  el,
  expectAbsent,
  hostCard,
  launcherRequests,
  menuItemIds,
  mock,
  openAppMenu,
  openHost,
  openNewHostPanel,
  openSubmenu,
  marker,
  pressAlt,
  pressCtrl,
  pressCtrlShift,
  pressEscape,
  restartApp,
  runCommand,
  setField,
  startFresh,
  tabIds,
  unlock,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForAppExit,
  waitForBoot,
  waitForHostsReady,
  waitForLocalShell,
  waitForLockState,
  waitForNewTab,
  waitForOutputLine,
  waitForPaneState,
  waitForSaved,
  waitForSelectedTab,
  waitForTabClosed,
  waitForTabCount,
  type Session,
} from '../helpers/index.ts';

const TOP_LEVEL = [
  'app-menu-vaults',
  'app-menu-settings',
  'app-menu-new-local-terminal',
  'app-menu-edit',
  'app-menu-view',
  'app-menu-window',
  'app-menu-help',
  'app-menu-lock',
  'app-menu-exit',
];

const HIDDEN = [
  'app-menu-sftp', // stage 4
  'app-menu-new-serial', // stage 9
  'app-menu-help-check-updates', // stage 9
];

/** Opens a local terminal from the app menu and waits for its shell; returns the tab id. */
async function newLocalTerminal(): Promise<string> {
  const before = await tabIds();
  await appMenu('app-menu-new-local-terminal');
  const tab = await waitForNewTab(before);
  await waitForSelectedTab(tab);
  await waitForPaneState('connected', { timeout: 30_000 });
  await waitForLocalShell();
  return tab;
}

async function zoom(): Promise<number> {
  return browser.execute(() => {
    const value = getComputedStyle(document.documentElement).zoom;
    return value === '' || value === 'normal' ? 1 : Number.parseFloat(value);
  });
}

async function expectZoom(expected: number): Promise<void> {
  await browser.waitUntil(async () => Math.abs((await zoom()) - expected) < 0.001, {
    timeout: 5_000,
    timeoutMsg: `expected <html> zoom ${expected}`,
  });
}

describe('UI 1.4 App menu', () => {
  let session: Session;
  /** Window handles of w-1 and of the w-2 opened with Window > New window. */
  let first = '';
  let second = '';

  before(async function () {
    session = await startFresh();
    if ((await appOs()) === 'macos') {
      // macOS uses the native menu bar (manual checklist).
      this.skip();
    }
  });

  it('lists the stage-2 items in order', async () => {
    await openAppMenu();
    expect(await menuItemIds('app-menu')).toEqual(TOP_LEVEL);
  });

  it('writes shortcut hints as symbols', async () => {
    expect(await el('app-menu-vaults').getText()).toContain('Ctrl+1');
    expect(await el('app-menu-settings').getText()).toContain('Ctrl+,');
    expect(await el('app-menu-new-local-terminal').getText()).toContain('Ctrl+L');
  });

  it('lists the Edit, View, Window and Help submenus', async () => {
    await openSubmenu('app-menu-edit');
    expect(await menuItemIds('app-menu-edit-menu')).toEqual([
      'app-menu-edit-undo',
      'app-menu-edit-redo',
      'app-menu-edit-cut',
      'app-menu-edit-copy',
      'app-menu-edit-paste',
      'app-menu-edit-select-all',
    ]);
    await openSubmenu('app-menu-view');
    expect(await menuItemIds('app-menu-view-menu')).toEqual([
      'app-menu-view-zoom-in',
      'app-menu-view-zoom-out',
      'app-menu-view-zoom-reset',
      'app-menu-view-fullscreen',
      'app-menu-view-toggle-sidebar',
    ]);
    await openSubmenu('app-menu-window');
    expect(await menuItemIds('app-menu-window-menu')).toEqual([
      'app-menu-window-new-window',
      'app-menu-window-next-tab',
      'app-menu-window-previous-tab',
      'app-menu-window-close-tab',
    ]);
    expect(await el('app-menu-window-next-tab').getText()).toContain('Alt+');
    expect(await el('app-menu-window-close-tab').getText()).toContain('Ctrl+Shift+W');
    await openSubmenu('app-menu-help');
    expect(await menuItemIds('app-menu-help-menu')).toEqual([
      'app-menu-help-documentation',
      'app-menu-help-report-issue',
      'app-menu-help-about',
    ]);
    await expectAbsent(['app-menu-help-check-updates']);
  });

  it('does not render the items of later stages', async () => {
    await expectAbsent(HIDDEN);
    await closeAppMenu();
  });

  it('opens Settings from the menu and with Ctrl+,', async () => {
    await appMenu('app-menu-settings');
    await waitFor('settings-modal');
    await pressEscape();
    await waitForAbsent('settings-modal');
    await pressCtrl(',');
    await waitFor('settings-modal');
    await closeSettings();
  });

  it('keeps the Vaults tab active with Vaults', async () => {
    await appMenu('app-menu-vaults');
    await expect(el('titlebar-tab-vaults')).toHaveAttribute('aria-selected', 'true');
    await waitForAbsent('app-menu');
  });

  it('opens the documentation and the issue tracker through the launcher', async () => {
    const before = (await launcherRequests()).length;
    await appMenu('app-menu-help', 'app-menu-help-documentation');
    await appMenu('app-menu-help', 'app-menu-help-report-issue');
    await browser.waitUntil(async () => (await launcherRequests()).length === before + 2, {
      timeout: 5_000,
      timeoutMsg: 'the launcher did not receive two URLs',
    });
    const [documentation, issues] = (await launcherRequests()).slice(before);
    expect(documentation).toMatch(/^https:\/\/github\.com\/proxyboy228\/shellpp/);
    expect(issues).toMatch(/^https:\/\/github\.com\/proxyboy228\/shellpp\/issues/);
  });

  it('shows the version in About', async () => {
    await appMenu('app-menu-help', 'app-menu-help-about');
    await waitFor('about-dialog');
    // WebDriver reports no text while the dialog's fade-in is still running.
    await browser.waitUntil(async () => (await el('about-dialog-version').getText()).includes(appVersion()), {
      timeout: 5_000,
      timeoutMsg: `About does not show the version ${appVersion()}`,
    });
    await el('about-dialog-close').click();
    await waitForAbsent('about-dialog');
  });

  it('changes the interface zoom from View', async () => {
    await expectZoom(1);
    await appMenu('app-menu-view', 'app-menu-view-zoom-in');
    await expectZoom(1.1);
    await appMenu('app-menu-view', 'app-menu-view-zoom-in');
    await expectZoom(1.2);
    await appMenu('app-menu-view', 'app-menu-view-zoom-out');
    await expectZoom(1.1);
    await appMenu('app-menu-view', 'app-menu-view-zoom-reset');
    await expectZoom(1);
  });

  it('pastes the clipboard text and cuts the selection with Edit', async () => {
    const text = `paste-${Date.now()}`;
    await mock({ kind: 'clipboard_write', text });
    await el('hosts-search-input').click();
    await appMenu('app-menu-edit', 'app-menu-edit-paste');
    await browser.waitUntil(async () => (await valueOf('hosts-search-input')) === text, {
      timeout: 5_000,
      timeoutMsg: 'Edit > Paste did not insert the clipboard text',
    });
    await appMenu('app-menu-edit', 'app-menu-edit-select-all');
    const selected = await browser.execute(() => {
      const input = document.activeElement as HTMLInputElement | null;
      return input ? (input.selectionEnd ?? 0) - (input.selectionStart ?? 0) : -1;
    });
    expect(selected).toBe(text.length);
    await appMenu('app-menu-edit', 'app-menu-edit-cut');
    await browser.waitUntil(async () => (await valueOf('hosts-search-input')) === '', {
      timeout: 5_000,
      timeoutMsg: 'Edit > Cut did not remove the selection',
    });
  });

  it('copies from one field and pastes into another with Edit', async () => {
    // Edit > Copy and Paste both go through the app clipboard (stage plan
    // 6.7), which is the e2e mock here.
    const text = `copy-${Date.now()}`;
    await el('hosts-search-input').setValue(text);
    await appMenu('app-menu-edit', 'app-menu-edit-select-all');
    await appMenu('app-menu-edit', 'app-menu-edit-copy');
    expect(await valueOf('hosts-search-input')).toBe(text);
    await browser.waitUntil(async () => (await clipboardText()) === text, {
      timeout: 5_000,
      timeoutMsg: 'Edit > Copy did not put the selected text on the app clipboard',
    });
    await openNewHostPanel();
    await el('host-panel-label').click();
    await appMenu('app-menu-edit', 'app-menu-edit-paste');
    await browser.waitUntil(async () => (await valueOf('host-panel-label')) === text, {
      timeout: 5_000,
      timeoutMsg: 'Edit > Paste did not insert the clipboard text into the second field',
    });
    await closePanel('host-panel');
    await el('hosts-search-clear').click();
  });

  it('opens a second, equal window with Window > New window', async () => {
    first = await browser.getWindowHandle();
    await appMenu('app-menu-window', 'app-menu-window-new-window');
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 2, {
      timeout: 20_000,
      timeoutMsg: 'the second window did not open',
    });
    second = (await browser.getWindowHandles()).find((handle) => handle !== first) ?? '';
    expect(second).not.toBe('');
    await browser.switchToWindow(second);
    await waitForBoot();
    await expect(el('app-root')).toHaveAttribute('data-window-label', 'w-2');
    await waitForLockState('unlocked');
    await waitForHostsReady();
  });

  it('shows data changes made in one window in the other at once', async () => {
    await browser.switchToWindow(first);
    const id = await createHost({ address: 'multi.example.com' });
    await browser.switchToWindow(second);
    await hostCard(id).waitForExist({ timeout: 10_000, timeoutMsg: 'the new host did not appear in w-2' });

    // An edit in the second window reaches the first.
    await openHost(id);
    await setField('host-panel-label', 'Edited in w-2');
    await waitForSaved('host-panel');
    await closePanel('host-panel');
    await browser.switchToWindow(first);
    await browser.waitUntil(async () => (await cardText('host-card', id, 'host-card-title')) === 'Edited in w-2', {
      timeout: 10_000,
      timeoutMsg: 'the label edited in w-2 did not reach w-1',
    });
  });

  it('shows settings changes made in one window in the other at once', async () => {
    await browser.switchToWindow(first);
    await expect(el('hosts-screen')).toHaveAttribute('data-layout', 'grid');
    await el('hosts-view-toggle-list').click();
    await expect(el('hosts-screen')).toHaveAttribute('data-layout', 'list');
    await browser.switchToWindow(second);
    await el('hosts-screen', '[data-layout="list"]').waitForExist({
      timeout: 10_000,
      timeoutMsg: 'the list layout chosen in w-1 did not reach w-2',
    });

    await el('hosts-view-toggle-grid').click();
    await browser.switchToWindow(first);
    await el('hosts-screen', '[data-layout="grid"]').waitForExist({
      timeout: 10_000,
      timeoutMsg: 'the grid layout chosen in w-2 did not reach w-1',
    });
  });

  it('closes the second window', async () => {
    await browser.switchToWindow(second);
    await el('titlebar-window-close').click();
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 1, {
      timeout: 10_000,
      timeoutMsg: 'the second window did not close',
    });
    await browser.switchToWindow(first);
  });

  it('opens a local terminal with New Local Terminal', async () => {
    await newLocalTerminal();
    const value = marker('shellpp');
    await runCommand(`echo ${value}`);
    await waitForOutputLine(value, { timeout: 20_000 });
  });

  it('opens a local terminal with Ctrl+Shift+L from the terminal', async () => {
    const before = await tabIds();
    await pressCtrlShift('l');
    const tab = await waitForNewTab(before);
    await waitForSelectedTab(tab);
    await waitForPaneState('connected', { timeout: 30_000 });
    await waitForLocalShell();
    await waitForTabCount(2);
  });

  it('moves between tabs with Window > Next tab and Previous tab', async () => {
    const [first, second] = await tabIds();
    await waitForSelectedTab(second);
    await appMenu('app-menu-window', 'app-menu-window-previous-tab');
    await waitForSelectedTab(first);
    await appMenu('app-menu-window', 'app-menu-window-next-tab');
    await waitForSelectedTab(second);
    // The same with Alt+Left and Alt+Right.
    await pressAlt(Key.ArrowLeft);
    await waitForSelectedTab(first);
    await pressAlt(Key.ArrowRight);
    await waitForSelectedTab(second);
  });

  it('closes the active tab with Window > Close tab', async () => {
    const [first, second] = await tabIds();
    await appMenu('app-menu-window', 'app-menu-window-close-tab');
    await waitForTabClosed(second);
    await waitForSelectedTab(first);
    await appMenu('app-menu-window', 'app-menu-window-close-tab');
    await waitForTabClosed(first);
    await waitForSelectedTab('vaults');
  });

  it('locks the app with Lock', async () => {
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await waitFor('unlock-screen');
    await expectAbsent(['vaults-tab']);
    await unlock(session.password);
  });

  it('exits the app with Exit (no question without active sessions)', async () => {
    expect(await tabIds()).toEqual([]);
    await appMenu('app-menu-exit');
    await waitForAppExit();
    // Start the app again so the session ends cleanly; the data survived.
    const state = await restartApp();
    expect(state).toBe('locked');
  });
});
