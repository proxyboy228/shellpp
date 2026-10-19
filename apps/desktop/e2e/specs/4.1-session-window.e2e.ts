// UI 4.1 Session window: while connecting the tab shows the connection
// screen (UI 4.8); then the terminal fills the area below the title bar
// without the Vaults sidebar, shows only what the server sends (MOTD first),
// has the input focus, uses the host's terminal theme or the default
// (Graphite) and reports TERM xterm-256color and its size to the server.

import {
  connectHost,
  createSshHost,
  el,
  existsInPane,
  expectAbsent,
  hostKey,
  marker,
  paneSelector,
  pressEnter,
  rectOf,
  runCommand,
  sessionTab,
  startFresh,
  terminalRenderer,
  terminalSize,
  terminalText,
  terminalTheme,
  tid,
  typeInTerminal,
  waitAbsentInPane,
  waitForActivePane,
  waitForOutputLine,
  waitForSelectedTab,
  waitForShell,
  waitInPane,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

describe('UI 4.1 Session window', () => {
  const motd = marker('motd');
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;
  let paperHostId: string;

  before(async () => {
    await startFresh();
    sshd = await startSshd({ motd: `${motd}\r\n` });
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'Session host' });
    paperHostId = await createSshHost(sshd, user, { label: 'Paper host', theme: 'paper' });
  });

  after(async () => {
    await stopAllSshd();
  });

  it('shows the connection screen while connecting, then the terminal', async () => {
    const tab = await connectHost(hostId);
    expect(tab).not.toBeNull();
    await waitForSelectedTab(tab as string);
    // The first connection stops at the host key question (UI 10.2), so the
    // connection screen stays up until it is answered.
    await waitInPane('connecting-view');
    await waitInPane('host-key-dialog');
    await hostKey('accept');
    await waitForShell(user.name);
    await waitAbsentInPane('connecting-view');
    expect(await existsInPane('terminal-view')).toBe(true);
    await expect(sessionTab(tab as string)).toHaveAttribute('data-state', 'connected');
  });

  it('fills the area below the title bar, without the Vaults sidebar', async () => {
    await expect(el('terminal-host')).toBeDisplayed();
    await expect(el('vaults-tab')).not.toBeDisplayed();
    await expect(el('sidebar')).not.toBeDisplayed();
    // The terminal sidebar (UI 4.2) is closed by default.
    await expectAbsent(['terminal-sidebar']);
    const bar = await rectOf(tid('titlebar'));
    const host = await rectOf(tid('terminal-host'));
    const viewport = await browser.execute(() => ({
      width: document.documentElement.clientWidth,
      height: document.documentElement.clientHeight,
    }));
    expect(bar).not.toBeNull();
    expect(host).not.toBeNull();
    expect(host!.y).toBeGreaterThanOrEqual(bar!.y + bar!.height - 1);
    expect(Math.abs(host!.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(host!.width - viewport.width)).toBeLessThanOrEqual(2);
    expect(Math.abs(host!.y + host!.height - viewport.height)).toBeLessThanOrEqual(2);
    // Only a small inner padding around the single pane.
    const pane = await rectOf(paneSelector(await waitForActivePane()));
    expect(pane!.width).toBeGreaterThan(host!.width - 40);
    expect(pane!.height).toBeGreaterThan(host!.height - 40);
  });

  it('has the input focus in the terminal right away', async () => {
    const value = marker('focus');
    // No click into the terminal first.
    await typeInTerminal(`echo ${value}`, { focus: false });
    await pressEnter();
    await waitForOutputLine(value);
  });

  it('shows only what the server sends, starting with the MOTD', async () => {
    const lines = (await terminalText()).split('\n').filter((line) => line.trim() !== '');
    expect(lines[0]?.trim()).toBe(motd);
  });

  it('reports TERM xterm-256color and the terminal size to the server', async () => {
    await runCommand('term');
    await waitForOutputLine('xterm-256color');
    const size = await terminalSize();
    expect(size).not.toBeNull();
    await runCommand('size');
    await waitForOutputLine(`${size!.cols}x${size!.rows}`);
  });

  it('renders with WebGL or with the DOM renderer', async () => {
    expect(['webgl', 'dom']).toContain(await terminalRenderer());
  });

  it('uses the default theme when the host has none', async () => {
    expect(await terminalTheme()).toBe('graphite');
  });

  it('uses the terminal theme of the host', async () => {
    const tab = await connectHost(paperHostId);
    // The host key was saved by the first connection: no question.
    await waitForSelectedTab(tab as string);
    await waitForShell(user.name);
    expect(await terminalTheme()).toBe('paper');
  });
});
