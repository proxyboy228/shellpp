// UI 4.2 Terminal sidebar: the title bar toggle (only on a session tab)
// opens the sidebar on the right and the terminal narrows; Tools searches the
// output with previous/next and a match count (Ctrl+Shift+F); History lists
// typed commands newest first, inserts without running, deletes entries and
// clears the host's history, and never records secrets (password prompts,
// secret-looking commands, the alternate screen; docs/security.md 10.1-10.2);
// Appearance changes the font size (global, persisted) and the theme (applied
// to the session and saved in the host). Snippets, Autocomplete and the
// "save as snippets" hint belong to later stages (hidden).

import crypto from 'node:crypto';
import { Key } from 'webdriverio';
import {
  activateTab,
  activateVaults,
  clearAndType,
  closePanel,
  closeTerminalSidebar,
  count,
  countLines,
  createSshHost,
  el,
  expectAbsent,
  expectPresent,
  focusTerminal,
  historyCommands,
  historyItemId,
  inPane,
  lastNonEmptyLine,
  marker,
  openHostPanel,
  openSidebarTab,
  openSshSession,
  pressCtrlShift,
  rectOf,
  restartAndUnlock,
  runCommand,
  startFresh,
  terminalSize,
  terminalText,
  terminalTheme,
  tid,
  typeInTerminal,
  pressEnter,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForActivePane,
  waitForFocus,
  waitForHistory,
  waitForLastLine,
  waitForOutputLine,
  waitForTerminalText,
  type Session,
} from '../helpers/index.ts';
import { generatedSecret, startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

const HIDDEN_TABS = ['terminal-sidebar-tab-snippets']; // stage 5
const HIDDEN_TOOLS = ['terminal-autocomplete']; // later stage (10)
const HIDDEN_HISTORY = ['history-snippet-hint']; // stage 5

async function viewWidth(): Promise<number> {
  const pane = await waitForActivePane();
  return (await rectOf(inPane(pane, 'terminal-view')))?.width ?? 0;
}

describe('UI 4.2 Terminal sidebar', () => {
  let session: Session;
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;
  let tabId: string;
  const one = marker('one');
  const two = marker('two');

  before(async () => {
    session = await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'Sidebar host' });
  });

  after(async () => {
    await stopAllSshd();
  });

  it('shows the toggle only while a session tab is active', async () => {
    await expectAbsent(['titlebar-terminal-sidebar-toggle']);
    tabId = await openSshSession(hostId, user.name);
    await expect(el('titlebar-terminal-sidebar-toggle')).toBeDisplayed();
    await expect(el('titlebar-terminal-sidebar-toggle')).toHaveAttribute('aria-pressed', 'false');
    await activateVaults();
    await expectAbsent(['titlebar-terminal-sidebar-toggle']);
    await activateTab(tabId);
    await waitFor('titlebar-terminal-sidebar-toggle');
  });

  it('opens the sidebar on the right and narrows the terminal', async () => {
    const before = await viewWidth();
    await el('titlebar-terminal-sidebar-toggle').click();
    await waitFor('terminal-sidebar');
    await expect(el('terminal-sidebar')).toBeDisplayed();
    await expect(el('titlebar-terminal-sidebar-toggle')).toHaveAttribute('aria-pressed', 'true');
    await browser.waitUntil(async () => (await viewWidth()) < before - 50, {
      timeout: 5_000,
      timeoutMsg: 'the terminal did not narrow when the sidebar opened',
    });
    const pane = await waitForActivePane();
    const sidebar = await rectOf(tid('terminal-sidebar'));
    const view = await rectOf(inPane(pane, 'terminal-view'));
    // On the right, next to the terminal, not over it.
    expect(sidebar!.x).toBeGreaterThanOrEqual(view!.x + view!.width - 1);
    await expectPresent(['terminal-sidebar-tab-tools', 'terminal-sidebar-tab-history', 'terminal-sidebar-tab-appearance']);
    await expectAbsent(HIDDEN_TABS);

    await closeTerminalSidebar();
    await expect(el('titlebar-terminal-sidebar-toggle')).toHaveAttribute('aria-pressed', 'false');
    await browser.waitUntil(async () => Math.abs((await viewWidth()) - before) <= 2, {
      timeout: 5_000,
      timeoutMsg: 'the terminal did not widen again when the sidebar closed',
    });
  });

  it('searches the terminal output from Tools with previous and next', async () => {
    await runCommand('lines 200');
    await waitForOutputLine('line 200');
    await openSidebarTab('tools');
    await expectAbsent(HIDDEN_TOOLS);
    await clearAndType('terminal-search-input', 'line 15');
    const results = el('terminal-search-results');
    // "line 15" and "line 150" ... "line 159".
    await browser.waitUntil(async () => (await results.getAttribute('data-count')) === '11', {
      timeout: 5_000,
      timeoutMsg: 'the search did not report 11 matches',
    });
    const first = await results.getAttribute('data-index');
    await el('terminal-search-next').click();
    await browser.waitUntil(async () => (await results.getAttribute('data-index')) !== first, {
      timeout: 5_000,
      timeoutMsg: 'Next did not move to another match',
    });
    await el('terminal-search-prev').click();
    await browser.waitUntil(async () => (await results.getAttribute('data-index')) === first, {
      timeout: 5_000,
      timeoutMsg: 'Previous did not return to the first match',
    });
  });

  it('focuses the search with Ctrl+Shift+F from the terminal', async () => {
    await closeTerminalSidebar();
    await focusTerminal();
    await pressCtrlShift('f');
    await waitFor('terminal-sidebar', '[data-tab="tools"]');
    await waitForFocus('terminal-search-input');
  });

  it('lists typed commands in History, newest first', async () => {
    await runCommand(`echo ${one}`);
    await waitForOutputLine(one);
    await runCommand(`echo ${two}`);
    await waitForOutputLine(two);
    await openSidebarTab('history');
    await expectAbsent(HIDDEN_HISTORY);
    const commands = await waitForHistory((list) => list.includes(`echo ${one}`) && list.includes(`echo ${two}`));
    expect(commands.indexOf(`echo ${two}`)).toBeLessThan(commands.indexOf(`echo ${one}`));
  });

  it('does not record what is typed at a password prompt', async () => {
    const secret = generatedSecret('Sec-');
    const okBefore = countLines(await terminalText(), 'ok');
    await runCommand('read-secret');
    await waitForTerminalText('[sudo] password for');
    // The server does not echo this line.
    await typeInTerminal(secret);
    await pressEnter();
    await browser.waitUntil(async () => countLines(await terminalText(), 'ok') > okBefore, {
      timeout: 10_000,
      timeoutMsg: 'read-secret did not finish',
    });
    expect(await terminalText()).not.toContain(secret);
    // Not "secret": the history detector drops every command containing it (S5).
    const after = marker('after-prompt');
    await runCommand(`echo ${after}`);
    await waitForOutputLine(after);
    const commands = await waitForHistory((list) => list.includes(`echo ${after}`));
    expect(commands.some((command) => command.includes(secret))).toBe(false);
  });

  it('does not record commands that look like secrets', async () => {
    const command = `export TOKEN=${crypto.randomBytes(12).toString('hex')}`;
    await runCommand(command);
    await waitForTerminalText('export: command not found');
    const after = marker('after-token');
    await runCommand(`echo ${after}`);
    await waitForOutputLine(after);
    const commands = await waitForHistory((list) => list.includes(`echo ${after}`));
    expect(commands.some((entry) => entry.includes('TOKEN='))).toBe(false);
  });

  it('does not record commands typed in the alternate screen', async () => {
    const hidden = marker('alt');
    await runCommand('alt-screen');
    // The probe reads the normal buffer only: give the switch a moment.
    await browser.pause(1_000);
    await typeInTerminal(`echo ${hidden}`);
    await pressEnter();
    // `q` leaves the alternate screen of the test shell.
    await typeInTerminal('q', { focus: false });
    const after = marker('after-alt');
    await runCommand(`echo ${after}`);
    await waitForOutputLine(after);
    const commands = await waitForHistory((list) => list.includes(`echo ${after}`));
    expect(commands.some((entry) => entry.includes(hidden))).toBe(false);
  });

  it('inserts an entry into the terminal without running it', async () => {
    await openSidebarTab('history');
    const id = await historyItemId(`echo ${one}`);
    const outputs = countLines(await terminalText(), one);
    await $(`${tid('history-item', `[data-record-id="${id}"]`)} ${tid('history-item-command')}`).click();
    await waitForLastLine((line) => line.endsWith(`$ echo ${one}`), { what: `a prompt line ending with "echo ${one}"` });
    await browser.pause(1_000);
    expect(countLines(await terminalText(), one)).toBe(outputs);
    // Discard the inserted line.
    await focusTerminal();
    await browser.keys([Key.Control, 'c']);
    await waitForLastLine((line) => line.endsWith('$'), { what: 'a fresh prompt' });
    expect(lastNonEmptyLine(await terminalText())).not.toContain(one);
  });

  it('deletes a single entry', async () => {
    await openSidebarTab('history');
    const id = await historyItemId(`echo ${two}`);
    const item = el('history-item', `[data-record-id="${id}"]`);
    await item.moveTo();
    await $(`${tid('history-item', `[data-record-id="${id}"]`)} ${tid('history-item-delete')}`).click();
    await waitForAbsent('history-item', `[data-record-id="${id}"]`);
    expect(await historyCommands()).toContain(`echo ${one}`);
  });

  it('clears the history of the host after confirmation', async () => {
    await el('history-clear').click();
    await waitFor('history-clear-dialog');
    await el('history-clear-dialog-cancel').click();
    await waitForAbsent('history-clear-dialog');
    expect(await count('history-item')).toBeGreaterThan(0);

    await el('history-clear').click();
    await waitFor('history-clear-dialog');
    await el('history-clear-dialog-confirm').click();
    await waitForAbsent('history-clear-dialog');
    await waitFor('history-empty');
    expect(await count('history-item')).toBe(0);
  });

  it('changes the font size from Appearance and keeps it after a restart', async () => {
    await openSidebarTab('appearance');
    await el('appearance-font').click();
    await waitFor('appearance-font-panel');
    await expectPresent(['appearance-font-family', 'appearance-font-size', 'appearance-line-height', 'appearance-font-back']);
    const initial = Number(await valueOf('appearance-font-size'));
    expect(initial).toBe(14);
    const colsBefore = (await terminalSize())?.cols ?? 0;

    await el('appearance-font-size-increase').click();
    await browser.waitUntil(async () => Number(await valueOf('appearance-font-size')) === initial + 1, {
      timeout: 5_000,
      timeoutMsg: '+ did not increase the font size',
    });
    // A larger font fits fewer columns.
    await browser.waitUntil(async () => ((await terminalSize())?.cols ?? 0) < colsBefore, {
      timeout: 5_000,
      timeoutMsg: 'the terminal did not refit to the larger font',
    });
    await el('appearance-font-size-decrease').click();
    await browser.waitUntil(async () => Number(await valueOf('appearance-font-size')) === initial, {
      timeout: 5_000,
      timeoutMsg: '- did not decrease the font size',
    });
    await el('appearance-font-size-increase').click();
    await browser.waitUntil(async () => Number(await valueOf('appearance-font-size')) === initial + 1, { timeout: 5_000 });
    await el('appearance-font-back').click();
    await waitFor('appearance-theme-list');

    await browser.pause(500);
    await restartAndUnlock(session.password);
    tabId = await openSshSession(hostId, user.name, { hostKey: 'none' });
    await openSidebarTab('appearance');
    await el('appearance-font').click();
    await waitFor('appearance-font-panel');
    expect(Number(await valueOf('appearance-font-size'))).toBe(initial + 1);
    await el('appearance-font-back').click();
    await waitFor('appearance-theme-list');
  });

  it('applies a theme to the session and saves it in the host', async () => {
    await openSidebarTab('appearance');
    expect(await count('appearance-theme-option')).toBeGreaterThanOrEqual(2);
    expect(await count('appearance-theme-option-preview')).toBe(await count('appearance-theme-option'));
    await expect(el('appearance-theme-option', '[data-theme="graphite"]')).toHaveAttribute('aria-selected', 'true');
    await el('appearance-theme-option', '[data-theme="paper"]').click();
    await browser.waitUntil(async () => (await terminalTheme()) === 'paper', {
      timeout: 5_000,
      timeoutMsg: 'the Paper theme was not applied to the terminal',
    });
    await expect(el('appearance-theme-option', '[data-theme="paper"]')).toHaveAttribute('aria-selected', 'true');
    await expect(el('appearance-theme-option', '[data-theme="graphite"]')).toHaveAttribute('aria-selected', 'false');

    await openHostPanel(hostId);
    const toggle = await waitFor('host-panel-show-more');
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
      await toggle.click();
    }
    await browser.waitUntil(async () => (await el('host-panel-theme').getText()).includes('Paper'), {
      timeout: 10_000,
      timeoutMsg: 'the host panel does not show the Paper theme',
    });
    await closePanel('host-panel');
  });

  it('does not render the elements of later stages', async () => {
    await activateTab(tabId);
    await openSidebarTab('tools');
    await expectAbsent([...HIDDEN_TABS, ...HIDDEN_TOOLS]);
    await openSidebarTab('history');
    await expectAbsent(HIDDEN_HISTORY);
  });
});
