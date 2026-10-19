// Terminal helpers (UI 4.1-4.7; stage 2 plan, sections 8.1 and 8.3).
//
// xterm.js draws on a canvas (or with its DOM renderer), so terminal text is
// read through the e2e probe `window.__SHELLPP_E2E__.terminalText(paneId)`
// that e2e builds register per pane. Panes are found by test id
// (`terminal-host[data-active-tab]` -> `terminal-tab-panel[data-tab-id]` ->
// `terminal-pane[data-active="true"]`). Input goes through WebDriver key
// actions after a click on the pane's `terminal-view`, which focuses
// xterm.js's input element.

import crypto from 'node:crypto';
import { Key } from 'webdriverio';
import { pollUntil } from './app.ts';
import { tid } from './selectors.ts';

/** Default wait for terminal output and session states (stage 2 plan, 8.1). */
export const TERMINAL_TIMEOUT = 15_000;

/** A unique marker for output checks, e.g. `hi-3f9a0c12` (hex only: no `q`, no shell syntax). */
export function marker(prefix: string): string {
  return `${prefix}-${crypto.randomBytes(4).toString('hex')}`;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Matches `text` as a whole line of terminal text (output lines, not the echoed command). */
export function lineRegex(text: string): RegExp {
  return new RegExp(`(^|\\n)${escapeRegExp(text)}[ \\t]*(\\n|$)`);
}

/** The last non-empty line of terminal text (the screen may end with empty rows). */
export function lastNonEmptyLine(content: string): string {
  const lines = content.split('\n').filter((line) => line.trim() !== '');
  return lines.at(-1)?.trimEnd() ?? '';
}

/** Number of whole lines equal to `text`. */
export function countLines(content: string, text: string): number {
  return content.split('\n').filter((line) => line.trimEnd() === text).length;
}

/** CSS selector of a pane by id. */
export function paneSelector(paneId: string): string {
  return tid('terminal-pane', `[data-pane-id="${paneId}"]`);
}

/** CSS selector of an element with a test id inside a pane. */
export function inPane(paneId: string, id: string, extra = ''): string {
  return `${paneSelector(paneId)} ${tid(id, extra)}`;
}

/** Tab id of the active terminal tab (`terminal-host[data-active-tab]`), null while Vaults is active. */
export async function activeTerminalTabId(): Promise<string | null> {
  return browser.execute((host: string) => {
    const node = document.querySelector(host);
    const value = node?.getAttribute('data-active-tab');
    return value === undefined || value === null || value === '' ? null : value;
  }, tid('terminal-host'));
}

/**
 * Id of the active pane of the active tab (the first pane when none is marked
 * active), or null when no terminal tab is active.
 */
export async function activePaneId(): Promise<string | null> {
  return browser.execute((host: string, panel: string, pane: string) => {
    const tab = document.querySelector(host)?.getAttribute('data-active-tab');
    if (tab === undefined || tab === null || tab === '') {
      return null;
    }
    const panelNode = Array.from(document.querySelectorAll(panel)).find((node) => node.getAttribute('data-tab-id') === tab);
    if (!panelNode) {
      return null;
    }
    const active = panelNode.querySelector(`${pane}[data-active="true"]`) ?? panelNode.querySelector(pane);
    return active?.getAttribute('data-pane-id') ?? null;
  }, tid('terminal-host'), tid('terminal-tab-panel'), tid('terminal-pane'));
}

/** Waits for an active pane and returns its id. */
export async function waitForActivePane(timeout = TERMINAL_TIMEOUT): Promise<string> {
  let id: string | null = null;
  await pollUntil(
    async () => {
      id = await activePaneId();
      return id !== null;
    },
    { timeout, message: () => 'no active terminal pane (is a session tab active?)' },
  );
  return id as unknown as string;
}

export interface PaneInfo {
  id: string;
  sessionId: string | null;
  active: boolean;
  state: string | null;
  rect: { x: number; y: number; width: number; height: number };
}

/** Panes of a tab (default: the active tab) in document order. */
export async function paneInfos(tabId?: string): Promise<PaneInfo[]> {
  return browser.execute(
    (host: string, panel: string, pane: string, wanted: string | null) => {
      const tab = wanted ?? document.querySelector(host)?.getAttribute('data-active-tab') ?? null;
      if (tab === null || tab === '') {
        return [];
      }
      const panelNode = Array.from(document.querySelectorAll(panel)).find((node) => node.getAttribute('data-tab-id') === tab);
      if (!panelNode) {
        return [];
      }
      return Array.from(panelNode.querySelectorAll(pane)).map((node) => {
        const box = node.getBoundingClientRect();
        return {
          id: node.getAttribute('data-pane-id') ?? '',
          sessionId: node.getAttribute('data-session-id'),
          active: node.getAttribute('data-active') === 'true',
          state: node.getAttribute('data-state'),
          rect: { x: box.x, y: box.y, width: box.width, height: box.height },
        };
      });
    },
    tid('terminal-host'),
    tid('terminal-tab-panel'),
    tid('terminal-pane'),
    tabId ?? null,
  );
}

/** Pane ids of a tab (default: the active tab) in document order. */
export async function terminalPanes(tabId?: string): Promise<string[]> {
  return (await paneInfos(tabId)).map((pane) => pane.id);
}

/** Pane ids that have a registered terminal probe (registration order). */
export async function probedPanes(): Promise<string[]> {
  return browser.execute(() => window.__SHELLPP_E2E__?.terminalPanes() ?? []);
}

async function resolvePane(paneId?: string): Promise<string> {
  return paneId ?? (await waitForActivePane());
}

/** Text of a pane's terminal (default: the active pane); '' when the pane has no terminal yet. */
export async function terminalText(paneId?: string): Promise<string> {
  const id = await resolvePane(paneId);
  return browser.execute((pane: string) => window.__SHELLPP_E2E__?.terminalText(pane) ?? '', id);
}

/** Waits until the pane's terminal text matches; returns the text. */
export async function waitForTerminalText(
  match: string | RegExp,
  options: { paneId?: string; timeout?: number } = {},
): Promise<string> {
  let text = '';
  await pollUntil(
    async () => {
      const id = options.paneId ?? (await activePaneId());
      if (id === null) {
        return false;
      }
      text = await browser.execute((pane: string) => window.__SHELLPP_E2E__?.terminalText(pane) ?? '', id);
      return typeof match === 'string' ? text.includes(match) : match.test(text);
    },
    {
      timeout: options.timeout ?? TERMINAL_TIMEOUT,
      interval: 150,
      message: () => `terminal text does not match ${String(match)}; last lines:\n${text.split('\n').slice(-12).join('\n')}`,
    },
  );
  return text;
}

/** Waits until the pane's terminal shows a line equal to `text` (command output). */
export async function waitForOutputLine(text: string, options: { paneId?: string; timeout?: number } = {}): Promise<string> {
  return waitForTerminalText(lineRegex(text), options);
}

/** Focuses a pane's terminal (default: the active pane) by clicking its `terminal-view`. */
export async function focusTerminal(paneId?: string): Promise<string> {
  const id = await resolvePane(paneId);
  const view = $(inPane(id, 'terminal-view'));
  await view.waitForExist({ timeout: TERMINAL_TIMEOUT });
  await view.click();
  return id;
}

/** Types text into a pane's terminal (clicks its `terminal-view` first unless `focus: false`). */
export async function typeInTerminal(text: string, options: { paneId?: string; focus?: boolean } = {}): Promise<void> {
  if (options.focus !== false) {
    await focusTerminal(options.paneId);
  }
  if (text !== '') {
    await browser.keys(text);
  }
}

/** Presses Enter. */
export async function pressEnter(): Promise<void> {
  await browser.keys(Key.Enter);
}

/** Types a command and presses Enter. */
export async function runCommand(command: string, options: { paneId?: string; focus?: boolean } = {}): Promise<void> {
  await typeInTerminal(command, options);
  await pressEnter();
}

/** Runs `echo <marker>` and waits for the output line. */
export async function echoMarker(prefix: string, options: { paneId?: string; focus?: boolean } = {}): Promise<string> {
  const value = marker(prefix);
  await runCommand(`echo ${value}`, options);
  await waitForOutputLine(value, { paneId: options.paneId });
  return value;
}

/** `terminal-view[data-renderer]` of a pane: `webgl` or `dom`. */
export async function terminalRenderer(paneId?: string): Promise<string | null> {
  const id = await resolvePane(paneId);
  return $(inPane(id, 'terminal-view')).getAttribute('data-renderer');
}

/** `terminal-view[data-theme]` of a pane. */
export async function terminalTheme(paneId?: string): Promise<string | null> {
  const id = await resolvePane(paneId);
  return $(inPane(id, 'terminal-view')).getAttribute('data-theme');
}

/** Selection text of a pane's terminal (probe). */
export async function terminalSelection(paneId?: string): Promise<string> {
  const id = await resolvePane(paneId);
  return browser.execute((pane: string) => window.__SHELLPP_E2E__?.terminal(pane)?.selection() ?? '', id);
}

/** Columns and rows of a pane's terminal (probe). */
export async function terminalSize(paneId?: string): Promise<{ cols: number; rows: number } | null> {
  const id = await resolvePane(paneId);
  return browser.execute((pane: string) => window.__SHELLPP_E2E__?.terminal(pane)?.size() ?? null, id);
}

/**
 * Viewport coordinates of a terminal cell (column and row of the screen,
 * 0-based) in a pane, computed from the `terminal-view` content box and the
 * probe's size. Used for mouse selection.
 */
export async function cellPoint(column: number, row: number, paneId?: string): Promise<{ x: number; y: number }> {
  const id = await resolvePane(paneId);
  const point = await browser.execute(
    (selector: string, pane: string, col: number, line: number) => {
      const view = document.querySelector(selector) as HTMLElement | null;
      const size = window.__SHELLPP_E2E__?.terminal(pane)?.size();
      if (!view || !size) {
        return null;
      }
      const box = view.getBoundingClientRect();
      const style = getComputedStyle(view);
      const left = box.left + parseFloat(style.paddingLeft) + parseFloat(style.borderLeftWidth);
      const top = box.top + parseFloat(style.paddingTop) + parseFloat(style.borderTopWidth);
      const width = view.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      const height = view.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
      const cellWidth = width / size.cols;
      const cellHeight = height / size.rows;
      return { x: Math.round(left + (col + 0.5) * cellWidth), y: Math.round(top + (line + 0.5) * cellHeight) };
    },
    inPane(id, 'terminal-view'),
    id,
    column,
    row,
  );
  if (point === null) {
    throw new Error(`cannot compute the cell position in pane ${id}`);
  }
  return point;
}

/** Double click at viewport coordinates (word selection in the terminal). */
export async function doubleClickAt(point: { x: number; y: number }): Promise<void> {
  await browser
    .action('pointer', { parameters: { pointerType: 'mouse' } })
    .move({ origin: 'viewport', x: point.x, y: point.y })
    .down()
    .up()
    .pause(40)
    .down()
    .up()
    .perform();
}

/** Right click on a pane's terminal (paste, UI 4.3). */
export async function rightClickTerminal(paneId?: string): Promise<void> {
  const id = await resolvePane(paneId);
  const view = $(inPane(id, 'terminal-view'));
  await view.waitForExist({ timeout: TERMINAL_TIMEOUT });
  await view.click({ button: 'right' });
}

/** Ctrl+Shift+<key> (terminal and app shortcuts, UI 12.5). */
export async function pressCtrlShift(key: string): Promise<void> {
  await browser.keys([Key.Control, Key.Shift, key]);
}

/** Ctrl+Alt+<key> (pane focus with arrows, UI 4.7). */
export async function pressCtrlAlt(key: string): Promise<void> {
  await browser.keys([Key.Control, Key.Alt, key]);
}

/** Alt+<key> (next and previous tab with arrows, UI 12.5). */
export async function pressAlt(key: string): Promise<void> {
  await browser.keys([Key.Alt, key]);
}

/** Rendered size of an element with this selector. */
export async function rectOf(selector: string): Promise<{ x: number; y: number; width: number; height: number } | null> {
  return browser.execute((query: string) => {
    const node = document.querySelector(query);
    if (!node) {
      return null;
    }
    const box = node.getBoundingClientRect();
    return { x: box.x, y: box.y, width: box.width, height: box.height };
  }, selector);
}

/** Waits until the last non-empty line of a pane's terminal satisfies `predicate`; returns it. */
export async function waitForLastLine(
  predicate: (line: string) => boolean,
  options: { paneId?: string; timeout?: number; what?: string } = {},
): Promise<string> {
  let line = '';
  await pollUntil(
    async () => {
      line = lastNonEmptyLine(await terminalText(options.paneId));
      return predicate(line);
    },
    {
      timeout: options.timeout ?? TERMINAL_TIMEOUT,
      interval: 150,
      message: () => `the last terminal line "${line}" does not match ${options.what ?? 'the expectation'}`,
    },
  );
  return line;
}

/**
 * Waits until a local shell (UI 12.4: PowerShell on Windows, the login shell
 * elsewhere) has printed its prompt, then gives its line editor a moment to
 * start, so typed keys are not lost.
 */
export async function waitForLocalShell(options: { paneId?: string; timeout?: number } = {}): Promise<void> {
  await waitForLastLine((line) => /[>$#%]$/.test(line), {
    paneId: options.paneId,
    timeout: options.timeout ?? 30_000,
    what: 'a shell prompt',
  });
  await browser.pause(1_000);
}

// ---------------------------------------------------------------------------
// Terminal sidebar (UI 4.2)
// ---------------------------------------------------------------------------

export type TerminalSidebarTab = 'tools' | 'history' | 'appearance';

async function sidebarShown(): Promise<boolean> {
  const sidebar = $(tid('terminal-sidebar'));
  return (await sidebar.isExisting()) && sidebar.isDisplayed();
}

/** Opens the terminal sidebar with the title bar toggle (no-op when open). */
export async function openTerminalSidebar(): Promise<void> {
  if (!(await sidebarShown())) {
    const toggle = $(tid('titlebar-terminal-sidebar-toggle'));
    await toggle.waitForExist({ timeout: TERMINAL_TIMEOUT });
    await toggle.click();
  }
  await browser.waitUntil(sidebarShown, { timeout: 5_000, timeoutMsg: 'the terminal sidebar did not open' });
}

/** Closes the terminal sidebar with the title bar toggle (no-op when closed). */
export async function closeTerminalSidebar(): Promise<void> {
  if (await sidebarShown()) {
    await $(tid('titlebar-terminal-sidebar-toggle')).click();
  }
  await browser.waitUntil(async () => !(await sidebarShown()), {
    timeout: 5_000,
    timeoutMsg: 'the terminal sidebar did not close',
  });
}

/** Opens the terminal sidebar on a tab. */
export async function openSidebarTab(tab: TerminalSidebarTab): Promise<void> {
  await openTerminalSidebar();
  await $(tid(`terminal-sidebar-tab-${tab}`)).click();
  await $(tid('terminal-sidebar', `[data-tab="${tab}"]`)).waitForExist({ timeout: 5_000 });
}

/** Commands of the History tab, top to bottom (newest first). */
export async function historyCommands(): Promise<string[]> {
  return browser.execute(
    (item: string, command: string) =>
      Array.from(document.querySelectorAll(item)).map((node) =>
        ((node.querySelector(command) as HTMLElement | null)?.innerText ?? '').trim(),
      ),
    tid('history-item'),
    tid('history-item-command'),
  );
}

/** Waits until the History tab's commands satisfy `predicate`; returns them. */
export async function waitForHistory(
  predicate: (commands: string[]) => boolean,
  timeout = TERMINAL_TIMEOUT,
): Promise<string[]> {
  let commands: string[] = [];
  await pollUntil(
    async () => {
      commands = await historyCommands();
      return predicate(commands);
    },
    { timeout, message: () => `unexpected history: ${JSON.stringify(commands.slice(0, 20))}` },
  );
  return commands;
}

/** `data-record-id` of the first History entry with exactly this command. */
export async function historyItemId(command: string): Promise<string> {
  const id = await browser.execute(
    (item: string, part: string, wanted: string) => {
      const node = Array.from(document.querySelectorAll(item)).find(
        (candidate) => ((candidate.querySelector(part) as HTMLElement | null)?.innerText ?? '').trim() === wanted,
      );
      return node?.getAttribute('data-record-id') ?? null;
    },
    tid('history-item'),
    tid('history-item-command'),
    command,
  );
  if (id === null) {
    throw new Error(`no history entry "${command}"`);
  }
  return id;
}
