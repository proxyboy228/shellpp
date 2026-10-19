// UI 4.6 Session termination and connection drop: exit 0 closes the tab;
// a non-zero exit shows "Session ended with code N" with Reconnect and Close
// and no automatic attempts; a drop (connection closed without an exit
// status) shows the reason, keeps the output and reconnects automatically
// (1, 2, 4, ... s) with a countdown and Cancel; an attempt in progress shows
// its step with Cancel and Close; the neighbor rule picks the next active tab
// on every way of closing; Exit and closing the window ask first while
// sessions are active. Reconnect attempts run in real time.

import {
  appMenu,
  clickInPane,
  closeTab,
  createSshHost,
  echoMarker,
  el,
  existsInPane,
  focusTerminal,
  openSshSession,
  paneAttr,
  paneState,
  pollUntil,
  pressCtrlShift,
  restartApp,
  runCommand,
  selectedTab,
  sessionTab,
  startFresh,
  tabIds,
  terminalText,
  waitAbsentInPane,
  waitFor,
  waitForAbsent,
  waitForAppExit,
  waitForPaneState,
  waitForSelectedTab,
  waitForTabClosed,
  waitInPane,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

describe('UI 4.6 Session termination and connection drop', () => {
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;

  /** A connected session tab to the test host (its key is saved by the first test). */
  async function open(): Promise<string> {
    return openSshSession(hostId, user.name, { hostKey: 'none' });
  }

  before(async () => {
    await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'End host' });
  });

  after(async () => {
    await stopAllSshd();
  });

  it('closes the tab when the shell exits with code 0', async () => {
    const tab = await openSshSession(hostId, user.name, { hostKey: 'accept' });
    await runCommand('exit');
    await waitForTabClosed(tab);
    await waitForSelectedTab('vaults');
  });

  it('activates the left neighbor, else the right one, else Vaults', async () => {
    const a = await open();
    const b = await open();
    const c = await open();
    expect(await tabIds()).toEqual([a, b, c]);

    // The close button of the middle tab: the left neighbor becomes active.
    await closeTab(b);
    await waitForSelectedTab(a);
    // `exit` in the leftmost tab: no left neighbor, the right one.
    await runCommand('exit');
    await waitForTabClosed(a);
    await waitForSelectedTab(c);
    // Ctrl+Shift+W on the last tab: back to Vaults.
    await focusTerminal();
    await pressCtrlShift('w');
    await waitForTabClosed(c);
    await waitForSelectedTab('vaults');
  });

  it('shows "Session ended with code N" for a non-zero exit, without automatic attempts', async () => {
    const tab = await open();
    const kept = await echoMarker('kept');
    await runCommand('exit 3');
    await waitInPane('session-ended');
    expect(await paneAttr('session-ended', 'data-exit-status')).toBe('3');
    expect(await existsInPane('session-ended-message')).toBe(true);
    await expect(sessionTab(tab)).toHaveAttribute('data-state', 'ended');
    // The output stays.
    expect(await terminalText()).toContain(kept);
    await browser.pause(3_000);
    expect(await paneState()).toBe('ended');
    expect(await existsInPane('session-reconnecting')).toBe(false);

    await clickInPane('session-ended-reconnect');
    await waitForPaneState('connected');
    await waitAbsentInPane('session-ended');
    await echoMarker('reconnected');

    await runCommand('exit 4');
    await waitInPane('session-ended', '[data-exit-status="4"]');
    await clickInPane('session-ended-close');
    await waitForTabClosed(tab);
  });

  it('reconnects automatically after a drop and keeps the output', async () => {
    await open();
    const before = await echoMarker('before-drop');
    expect(await sshd.drop()).toBeGreaterThanOrEqual(1);
    // The first attempt starts after 1 s.
    await waitInPane('session-dropped', '', 10_000);
    expect(await paneAttr('session-dropped', 'data-code')).toBe('ssh_error');
    expect(await paneAttr('session-dropped', 'data-reason')).toBe('disconnected');
    expect(await existsInPane('session-dropped-message')).toBe(true);
    await waitForPaneState('connected', { timeout: 15_000 });
    await waitAbsentInPane('session-dropped');
    expect(await terminalText()).toContain(before);
    await echoMarker('after-drop');
  });

  it('counts down between failing attempts and stops them with Cancel', async () => {
    await sshd.setAccepting(false);
    await sshd.drop();
    await waitInPane('session-dropped', '', 10_000);
    // 1 s before the first attempt; once it fails (refused) the next one
    // waits 2 s, then 4 s.
    await pollUntil(async () => Number((await paneAttr('session-dropped-countdown', 'data-seconds').catch(() => null)) ?? 0) >= 2, {
      timeout: 20_000,
      message: () => 'the countdown did not move on to a later attempt',
    });
    expect(await existsInPane('session-dropped-cancel')).toBe(true);
    await clickInPane('session-dropped-cancel');
    await waitAbsentInPane('session-dropped-countdown');
    await waitAbsentInPane('session-dropped-cancel');
    expect(await existsInPane('session-dropped')).toBe(true);
    await browser.pause(5_000);
    expect(await paneState()).toBe('dropped');
    expect(await existsInPane('session-reconnecting')).toBe(false);

    await sshd.setAccepting(true);
    await clickInPane('session-dropped-reconnect');
    await waitForPaneState('connected');
    await waitAbsentInPane('session-dropped');
    await echoMarker('after-cancel');
  });

  it('stops a hanging reconnect attempt with the Cancel of its overlay', async () => {
    // New connections are accepted but never answered: the attempt hangs.
    await sshd.setHolding(true);
    expect(await sshd.drop()).toBeGreaterThanOrEqual(1);
    await waitInPane('session-reconnecting', '', 15_000);
    expect(await existsInPane('session-reconnecting-step')).toBe(true);
    expect(await existsInPane('session-reconnecting-close')).toBe(true);
    await clickInPane('session-reconnecting-cancel');
    await waitAbsentInPane('session-reconnecting');
    await waitInPane('session-dropped');
    expect(await existsInPane('session-dropped-countdown')).toBe(false);
    await browser.pause(3_000);
    expect(await paneState()).toBe('dropped');

    await sshd.setHolding(false);
    await sshd.drop();
    await clickInPane('session-dropped-reconnect');
    await waitForPaneState('connected');
    await echoMarker('after-hold');
  });

  it('closes a dropped session with Close', async () => {
    const tab = await selectedTab();
    expect(tab).not.toBe('vaults');
    await sshd.setAccepting(false);
    await sshd.drop();
    await waitInPane('session-dropped', '', 10_000);
    await clickInPane('session-dropped-close');
    await waitForTabClosed(tab as string);
    await sshd.setAccepting(true);
  });

  it('asks before Exit while a session is active', async () => {
    const tab = await open();
    await appMenu('app-menu-exit');
    const dialog = await waitFor('exit-confirm-dialog');
    await expect(dialog).toHaveAttribute('data-count', '1');
    await el('exit-confirm-dialog-cancel').click();
    await waitForAbsent('exit-confirm-dialog');
    expect((await browser.getWindowHandles()).length).toBe(1);
    await expect(sessionTab(tab)).toBeExisting();
  });

  it('asks before closing the window while a session is active', async () => {
    await el('titlebar-window-close').click();
    const dialog = await waitFor('window-close-confirm-dialog');
    await expect(dialog).toHaveAttribute('data-count', '1');
    await el('window-close-confirm-dialog-cancel').click();
    await waitForAbsent('window-close-confirm-dialog');
    expect(await tabIds()).toHaveLength(1);
  });

  it('exits after the confirmation', async () => {
    await appMenu('app-menu-exit');
    await waitFor('exit-confirm-dialog');
    await el('exit-confirm-dialog-confirm').click();
    await waitForAppExit();
    // Start the app again so the session ends cleanly.
    expect(await restartApp()).toBe('locked');
  });
});
