// UI 4.3 Mouse in the terminal (and the copy/paste keys of UI 12.5): right
// click pastes the clipboard; a multi-line paste asks first, with a preview
// and the line count; the paste filter strips control characters and
// bracketed paste markers (docs/security.md 12); copy on select is off by
// default; Ctrl+Shift+C copies the selection, Ctrl+Shift+A selects all and
// Ctrl+Shift+V pastes. The clipboard is the e2e mock.

import crypto from 'node:crypto';
import {
  cellPoint,
  clipboardText,
  createSshHost,
  doubleClickAt,
  el,
  expectAbsent,
  focusTerminal,
  marker,
  mock,
  openSshSession,
  pressCtrlShift,
  pressEnter,
  rightClickTerminal,
  runCommand,
  startFresh,
  terminalSelection,
  terminalSize,
  terminalText,
  waitFor,
  waitForAbsent,
  waitForOutputLine,
  waitForTerminalText,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

async function setClipboard(text: string): Promise<void> {
  await mock({ kind: 'clipboard_write', text });
}

describe('UI 4.3 Mouse in the terminal', () => {
  let sshd: Sshd;
  let user: SshdUser;

  before(async () => {
    await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    const hostId = await createSshHost(sshd, user, { label: 'Mouse host' });
    await openSshSession(hostId, user.name);
  });

  after(async () => {
    await stopAllSshd();
  });

  it('selects a word with a double click without copying it, and copies it with Ctrl+Shift+C', async () => {
    // Runs first, while the whole buffer fits on the screen (no scrollback),
    // so the output line's row is its index in the terminal text.
    const word = `copy${crypto.randomBytes(4).toString('hex')}`;
    await runCommand(`echo ${word}`);
    const text = await waitForOutputLine(word);
    const lines = text.split('\n');
    const size = await terminalSize();
    expect(size).not.toBeNull();
    const offset = Math.max(0, lines.length - size!.rows);
    const row = lines.findIndex((line) => line.trimEnd() === word) - offset;
    expect(row).toBeGreaterThanOrEqual(0);
    await setClipboard('');

    await doubleClickAt(await cellPoint(2, row));
    await browser.waitUntil(async () => (await terminalSelection()) === word, {
      timeout: 5_000,
      timeoutMsg: `a double click on row ${row} did not select "${word}"`,
    });
    // terminal.copy_on_select is off by default.
    await browser.pause(500);
    expect((await clipboardText()) ?? '').toBe('');

    await pressCtrlShift('c');
    await browser.waitUntil(async () => (await clipboardText()) === word, {
      timeout: 5_000,
      timeoutMsg: 'Ctrl+Shift+C did not copy the selection',
    });
  });

  it('selects everything with Ctrl+Shift+A', async () => {
    const value = marker('all');
    await runCommand(`echo ${value}`);
    await waitForOutputLine(value);
    await focusTerminal();
    await pressCtrlShift('a');
    await browser.waitUntil(async () => (await terminalSelection()).includes(value), {
      timeout: 5_000,
      timeoutMsg: 'Ctrl+Shift+A did not select the terminal content',
    });
    await pressCtrlShift('c');
    await browser.waitUntil(async () => ((await clipboardText()) ?? '').includes(value), {
      timeout: 5_000,
      timeoutMsg: 'Ctrl+Shift+C did not copy the whole selection',
    });
  });

  it('pastes a single line on right click without asking', async () => {
    const value = marker('paste');
    await setClipboard(`echo ${value}`);
    await rightClickTerminal();
    await waitForTerminalText(`$ echo ${value}`);
    await expectAbsent(['paste-confirm-dialog']);
    await focusTerminal();
    await pressEnter();
    await waitForOutputLine(value);
  });

  it('pastes with Ctrl+Shift+V', async () => {
    const value = marker('ctrl-v');
    await setClipboard(`echo ${value}`);
    await focusTerminal();
    await pressCtrlShift('v');
    await waitForTerminalText(`$ echo ${value}`);
    await pressEnter();
    await waitForOutputLine(value);
  });

  it('asks before pasting several lines and pastes only after confirmation', async () => {
    const first = marker('m1');
    const second = marker('m2');
    await setClipboard(`echo ${first}\necho ${second}`);

    await rightClickTerminal();
    await waitFor('paste-confirm-dialog');
    await expect(el('paste-confirm-lines')).toHaveAttribute('data-count', '2');
    await browser.waitUntil(async () => (await el('paste-confirm-preview').getText()).includes(`echo ${first}`), {
      timeout: 5_000,
      timeoutMsg: 'the paste preview does not show the text',
    });
    await el('paste-confirm-cancel').click();
    await waitForAbsent('paste-confirm-dialog');
    await browser.pause(500);
    expect(await terminalText()).not.toContain(`echo ${first}`);

    await rightClickTerminal();
    await waitFor('paste-confirm-dialog');
    await el('paste-confirm-paste').click();
    await waitForAbsent('paste-confirm-dialog');
    await waitForOutputLine(first);
    // The last line has no line break: it waits at the prompt.
    await waitForTerminalText(`$ echo ${second}`);
    await focusTerminal();
    await pressEnter();
    await waitForOutputLine(second);
  });

  it('strips control characters and bracketed paste markers from pasted text', async () => {
    const tail = crypto.randomBytes(4).toString('hex');
    await setClipboard(`echo ctl-\u001b[201~${tail}\u0007x`);
    await rightClickTerminal();
    await waitForTerminalText(`$ echo ctl-${tail}x`);
    await expectAbsent(['paste-confirm-dialog']);
    await focusTerminal();
    await pressEnter();
    await waitForOutputLine(`ctl-${tail}x`);
  });
});
