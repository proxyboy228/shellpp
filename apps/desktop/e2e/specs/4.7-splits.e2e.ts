// UI 4.7 Splits: Ctrl+Shift+D adds a pane to the right, Ctrl+Shift+E a
// pane below; a new pane shows host selection (search and Recent
// connections) and connects the chosen host in that pane; every pane runs its
// own session; the border between panes can be dragged; the active pane is
// highlighted and focus moves by click or Ctrl+Alt+arrows; a pane closes
// with its session, an empty pane with Escape, and the tab closes when its
// last session ends. Broadcast input is stage 9 (hidden).

import { Key } from 'webdriverio';
import {
  createSshHost,
  expectAbsent,
  focusTerminal,
  inPane,
  marker,
  openSshSession,
  openTabMenu,
  paneInfos,
  pressCtrlAlt,
  pressCtrlShift,
  pressEscape,
  runCommand,
  startFresh,
  terminalSize,
  terminalText,
  tid,
  waitForAbsent,
  waitForFocusAt,
  waitForOutputLine,
  waitForShell,
  waitForTabClosed,
  type PaneInfo,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

async function waitForPaneCount(n: number): Promise<PaneInfo[]> {
  let panes: PaneInfo[] = [];
  await browser.waitUntil(
    async () => {
      panes = await paneInfos();
      return panes.length === n;
    },
    { timeout: 10_000, timeoutMsg: `expected ${n} panes in the active tab, got ${panes.length}` },
  );
  return panes;
}

async function pane(id: string): Promise<PaneInfo> {
  const found = (await paneInfos()).find((candidate) => candidate.id === id);
  if (!found) {
    throw new Error(`pane ${id} is gone`);
  }
  return found;
}

async function waitForActive(id: string): Promise<void> {
  await browser.waitUntil(
    async () => {
      const panes = await paneInfos();
      return panes.filter((candidate) => candidate.active).map((candidate) => candidate.id).join() === id;
    },
    { timeout: 5_000, timeoutMsg: `expected pane ${id} to be the only active pane` },
  );
}

/**
 * A point on the divider whose box is taller than wide (between side-by-side
 * panes), a quarter of its height from the top: its middle can be the
 * junction with the divider of a split in the right column.
 */
async function verticalDividerPoint(): Promise<{ x: number; y: number }> {
  const point = await browser.execute((selector: string) => {
    const divider = Array.from(document.querySelectorAll(selector))
      .map((node) => node.getBoundingClientRect())
      .find((box) => box.width > 0 && box.height > box.width);
    return divider ? { x: Math.round(divider.x + divider.width / 2), y: Math.round(divider.y + divider.height / 4) } : null;
  }, tid('split-divider'));
  if (point === null) {
    throw new Error('no divider between side-by-side panes');
  }
  return point;
}

describe('UI 4.7 Splits', () => {
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;
  let tab: string;
  let left: string;
  let right: string;
  let lower: string;

  before(async () => {
    await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'Split host' });
    // This connection also puts the host into Recent connections.
    tab = await openSshSession(hostId, user.name);
  });

  after(async () => {
    await stopAllSshd();
  });

  it('adds a pane to the right with Ctrl+Shift+D and focuses its host search', async () => {
    left = await focusTerminal();
    await pressCtrlShift('d');
    const panes = await waitForPaneCount(2);
    const fresh = panes.find((candidate) => candidate.id !== left)!;
    right = fresh.id;
    const original = panes.find((candidate) => candidate.id === left)!;
    expect(fresh.sessionId).toBeNull();
    expect(fresh.rect.x).toBeGreaterThanOrEqual(original.rect.x + original.rect.width - 2);
    const search = inPane(right, 'pane-host-picker-search-input');
    await $(search).waitForExist();
    await waitForFocusAt(search);
    await $(inPane(right, 'pane-host-picker-recent')).waitForExist();
  });

  it('connects a host chosen from Recent connections in that pane', async () => {
    const item = $(inPane(right, 'pane-host-picker-recent-item', `[data-host-id="${hostId}"]`));
    await item.waitForExist({ timeout: 10_000 });
    await item.click();
    await waitForShell(user.name, { paneId: right });
    expect((await pane(right)).sessionId).not.toBeNull();
    await waitForActive(right);
  });

  it('adds a pane below with Ctrl+Shift+E and connects a host found by search', async () => {
    await focusTerminal(right);
    await pressCtrlShift('e');
    const panes = await waitForPaneCount(3);
    const fresh = panes.find((candidate) => candidate.id !== left && candidate.id !== right)!;
    lower = fresh.id;
    const upper = panes.find((candidate) => candidate.id === right)!;
    expect(fresh.rect.y).toBeGreaterThanOrEqual(upper.rect.y + upper.rect.height - 2);
    expect(Math.abs(fresh.rect.x - upper.rect.x)).toBeLessThanOrEqual(2);
    const search = inPane(lower, 'pane-host-picker-search-input');
    await waitForFocusAt(search);
    await $(search).addValue('Split host');
    const result = $(inPane(lower, 'pane-host-picker-result-host', `[data-host-id="${hostId}"]`));
    await result.waitForExist();
    await result.click();
    await waitForShell(user.name, { paneId: lower });
  });

  it('runs an independent session in every pane', async () => {
    const panes = await paneInfos();
    const sessions = panes.map((candidate) => candidate.sessionId);
    expect(sessions.every((id) => id !== null)).toBe(true);
    expect(new Set(sessions).size).toBe(3);

    const value = marker('left-only');
    await runCommand(`echo ${value}`, { paneId: left });
    await waitForOutputLine(value, { paneId: left });
    expect(await terminalText(right)).not.toContain(value);
    expect(await terminalText(lower)).not.toContain(value);

    // Each pane's PTY has the pane's own size.
    const size = await terminalSize(lower);
    await runCommand('size', { paneId: lower });
    await waitForOutputLine(`${size!.cols}x${size!.rows}`, { paneId: lower });
  });

  it('highlights the active pane and focuses a pane by click', async () => {
    await focusTerminal(lower);
    await waitForActive(lower);
    await focusTerminal(left);
    await waitForActive(left);
  });

  it('moves the focus between panes with Ctrl+Alt+arrows', async () => {
    await focusTerminal(left);
    await waitForActive(left);
    await pressCtrlAlt(Key.ArrowRight);
    await browser.waitUntil(
      async () => {
        const active = (await paneInfos()).find((candidate) => candidate.active)?.id;
        return active === right || active === lower;
      },
      { timeout: 5_000, timeoutMsg: 'Ctrl+Alt+Right did not move the focus to the right column' },
    );
    await pressCtrlAlt(Key.ArrowDown);
    await waitForActive(lower);
    await pressCtrlAlt(Key.ArrowUp);
    await waitForActive(right);
    // Typing goes to the focused pane.
    const value = marker('right-only');
    await runCommand(`echo ${value}`, { paneId: right, focus: false });
    await waitForOutputLine(value, { paneId: right });
    expect(await terminalText(left)).not.toContain(value);
    await pressCtrlAlt(Key.ArrowLeft);
    await waitForActive(left);
  });

  it('resizes the panes by dragging the border', async () => {
    const before = (await pane(left)).rect.width;
    const start = await verticalDividerPoint();
    await browser
      .action('pointer', { parameters: { pointerType: 'mouse' } })
      .move({ origin: 'viewport', x: start.x, y: start.y })
      .down()
      .pause(50)
      .move({ origin: 'pointer', x: -60, y: 0, duration: 100 })
      .move({ origin: 'pointer', x: -60, y: 0, duration: 100 })
      .pause(100)
      .up()
      .perform();
    await browser.waitUntil(async () => (await pane(left)).rect.width < before - 60, {
      timeout: 5_000,
      timeoutMsg: 'dragging the border did not make the left pane narrower',
    });
  });

  it('removes a pane when its session ends', async () => {
    await runCommand('exit', { paneId: lower });
    const panes = await waitForPaneCount(2);
    expect(panes.map((candidate) => candidate.id)).not.toContain(lower);
  });

  it('closes an empty pane with Escape in its host search', async () => {
    await focusTerminal(left);
    await pressCtrlShift('d');
    const panes = await waitForPaneCount(3);
    const fresh = panes.find((candidate) => candidate.id !== left && candidate.id !== right)!;
    await waitForFocusAt(inPane(fresh.id, 'pane-host-picker-search-input'));
    await pressEscape();
    const after = await waitForPaneCount(2);
    expect(after.map((candidate) => candidate.id).sort()).toEqual([left, right].sort());
  });

  it('does not offer Broadcast input (stage 9)', async () => {
    await openTabMenu(tab);
    await expectAbsent(['session-tab-menu-broadcast', 'session-tab-menu-multiplayer']);
    await pressEscape();
    await waitForAbsent('session-tab-menu');
  });

  it('closes the tab when its last session ends and only an empty pane is left', async () => {
    await runCommand('exit', { paneId: right });
    await waitForPaneCount(1);
    await focusTerminal(left);
    await pressCtrlShift('d');
    await waitForPaneCount(2);
    await runCommand('exit', { paneId: left });
    await waitForTabClosed(tab);
  });
});
