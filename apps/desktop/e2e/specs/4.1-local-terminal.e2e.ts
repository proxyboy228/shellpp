// Local terminal (stage 2 scope, docs/architecture.md 6.1; UI 1.4, 2.2,
// 4.1, 4.6, 12.4, 12.5): New Local Terminal opens the default local shell
// (PowerShell on Windows, the login shell elsewhere) in a new session tab
// with the terminal focused; Ctrl+L opens one outside the terminal and goes
// to the shell inside it, Ctrl+Shift+L works everywhere; `exit` closes the
// tab, a non-zero exit code keeps it with "Session ended with code N".
// Shell prompts and colors vary, so the checks look for markers only.

import { Key } from 'webdriverio';
import {
  activateVaults,
  appMenu,
  appOs,
  clickInPane,
  focusTerminal,
  marker,
  paneAttr,
  pressCtrl,
  pressCtrlShift,
  runCommand,
  sessionTab,
  sessionTabs,
  startFresh,
  tabIds,
  waitForLocalShell,
  waitForNewTab,
  waitForOutputLine,
  waitForPaneState,
  waitForSelectedTab,
  waitForTabClosed,
  waitForTabCount,
  waitInPane,
} from '../helpers/index.ts';

async function openFromMenu(): Promise<string> {
  const before = await tabIds();
  await appMenu('app-menu-new-local-terminal');
  const tab = await waitForNewTab(before);
  await waitForSelectedTab(tab);
  return tab;
}

describe('Local terminal', () => {
  let os: string | null;

  before(async () => {
    await startFresh();
    os = await appOs();
  });

  it('opens the default local shell in a new tab from New Local Terminal', async function () {
    if (os === 'macos') {
      // The app menu is the native menu bar on macOS (manual checklist).
      this.skip();
    }
    const tab = await openFromMenu();
    await expect(sessionTab(tab)).toHaveAttribute('data-kind', 'session');
    await waitForPaneState('connected', { timeout: 30_000 });
    await waitForLocalShell();
    // The terminal has the focus: type without clicking.
    const value = marker('shellpp');
    await runCommand(`echo ${value}`, { focus: false });
    await waitForOutputLine(value, { timeout: 20_000 });
    const title = (await sessionTabs()).find((candidate) => candidate.id === tab)?.title ?? '';
    expect(title.length).toBeGreaterThan(0);
  });

  it('closes the tab when the shell exits with 0', async () => {
    const [tab] = await tabIds();
    await runCommand('exit');
    await waitForTabClosed(tab, 20_000);
    await waitForSelectedTab('vaults');
  });

  it('opens a local terminal with Ctrl+L outside the terminal', async () => {
    await activateVaults();
    const before = await tabIds();
    await pressCtrl('l');
    const tab = await waitForNewTab(before);
    await waitForSelectedTab(tab);
    await waitForPaneState('connected', { timeout: 30_000 });
    await waitForLocalShell();
  });

  it('sends Ctrl+L to the shell when the terminal has the focus', async () => {
    const count = (await tabIds()).length;
    await focusTerminal();
    await browser.keys([Key.Control, 'l']);
    await browser.pause(1_000);
    expect((await tabIds()).length).toBe(count);
    // The shell is still usable after its clear-screen key.
    const value = marker('after-ctrl-l');
    await runCommand(`echo ${value}`);
    await waitForOutputLine(value, { timeout: 20_000 });
  });

  it('opens another local terminal with Ctrl+Shift+L from the terminal', async () => {
    const before = await tabIds();
    await focusTerminal();
    await pressCtrlShift('l');
    const tab = await waitForNewTab(before);
    await waitForSelectedTab(tab);
    await waitForPaneState('connected', { timeout: 30_000 });
    await waitForLocalShell();
    await waitForTabCount(before.length + 1);
  });

  it('keeps the tab with "Session ended with code N" for a non-zero exit code', async () => {
    const [, tab] = await tabIds();
    await runCommand('exit 3');
    await waitInPane('session-ended', '', 20_000);
    expect(await paneAttr('session-ended', 'data-exit-status')).toBe('3');
    await expect(sessionTab(tab)).toHaveAttribute('data-state', 'ended');
    await clickInPane('session-ended-close');
    await waitForTabClosed(tab);
  });
});
