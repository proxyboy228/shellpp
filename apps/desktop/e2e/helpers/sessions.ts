// Session helpers (UI 2.1, 2.4, 3.1, 4.4, 4.6, 4.8, 6, 10; stage 2 plan,
// section 8.1): saved hosts pointing at a test server, connect actions,
// host key dialogs and prompts, session tabs, known hosts seeding.

import crypto from 'node:crypto';
import { Key } from 'webdriverio';
import type { KnownHostAddInput } from '../../src/ipc/types.ts';
import type { GeneratedKey, Sshd, SshdUser } from '../support/sshd.ts';
import { clearAndType, mock, pollUntil, waitForHostsReady } from './app.ts';
import { closePanel, createHost, hostCard, openHost, search, waitForSaved } from './hosts.ts';
import { attrOfAll, el, exists, tid, waitFor, waitForAbsent } from './selectors.ts';
import { activePaneId, inPane, TERMINAL_TIMEOUT, waitForActivePane, waitForTerminalText } from './terminal.ts';

/** Time allowed for a connection to reach a state (real time on loopback, plus KDF-free UI work). */
export const CONNECT_TIMEOUT = 20_000;

/** The test shell's MOTD (tests/sshd shell contract). */
export const DEFAULT_MOTD = 'Welcome to shellpp-test-sshd';

/** The test shell's prompt for a user (tests/sshd shell contract), without the trailing space. */
export function shellPrompt(user: string): string {
  return `${user}@shellpp-test:~$`;
}

/** `[127.0.0.1]:<port>`: the OpenSSH display address of the test server (port is never 22). */
export function displayAddress(sshd: Sshd): string {
  return `[${sshd.address}]:${sshd.port}`;
}

/** Quick connect text `user@127.0.0.1:<port>` (or `127.0.0.1:<port>` without a user). */
export function quickTarget(sshd: Sshd, user?: string): string {
  return user === undefined ? `${sshd.address}:${sshd.port}` : `${user}@${sshd.address}:${sshd.port}`;
}

// ---------------------------------------------------------------------------
// Title bar tabs (UI 1.1)
// ---------------------------------------------------------------------------

export interface TabInfo {
  id: string;
  kind: string | null;
  state: string | null;
  selected: boolean;
  title: string;
}

/** Session and new tabs of this window, left to right. */
export async function sessionTabs(): Promise<TabInfo[]> {
  return browser.execute(
    (tab: string, title: string) =>
      Array.from(document.querySelectorAll(tab)).map((node) => ({
        id: node.getAttribute('data-tab-id') ?? '',
        kind: node.getAttribute('data-kind'),
        state: node.getAttribute('data-state'),
        selected: node.getAttribute('aria-selected') === 'true',
        title: ((node.querySelector(title) as HTMLElement | null)?.innerText ?? '').trim(),
      })),
    tid('titlebar-session-tab'),
    tid('titlebar-session-tab-title'),
  );
}

/** Ids of the session tabs, left to right. */
export async function tabIds(): Promise<string[]> {
  return (await sessionTabs()).map((tab) => tab.id);
}

/** Id of the selected title bar tab: `vaults` or a session tab id. */
export async function selectedTab(): Promise<string | null> {
  const vaults = await el('titlebar-tab-vaults').getAttribute('aria-selected');
  if (vaults === 'true') {
    return 'vaults';
  }
  return (await sessionTabs()).find((tab) => tab.selected)?.id ?? null;
}

/** Waits until the selected title bar tab is `vaults` or this tab id. */
export async function waitForSelectedTab(expected: string, timeout = TERMINAL_TIMEOUT): Promise<void> {
  let current: string | null = null;
  await pollUntil(
    async () => {
      current = await selectedTab();
      return current === expected;
    },
    { timeout, message: () => `expected the selected tab to be ${expected}, got ${String(current)}` },
  );
}

/** Waits until this window has exactly `n` session tabs; returns their ids. */
export async function waitForTabCount(n: number, timeout = TERMINAL_TIMEOUT): Promise<string[]> {
  let ids: string[] = [];
  await pollUntil(
    async () => {
      ids = await tabIds();
      return ids.length === n;
    },
    { timeout, message: () => `expected ${n} session tabs, got ${ids.length} (${ids.join(', ')})` },
  );
  return ids;
}

/** Waits for a tab that was not in `before` and returns its id. */
export async function waitForNewTab(before: string[], timeout = TERMINAL_TIMEOUT): Promise<string> {
  let fresh: string | undefined;
  await pollUntil(
    async () => {
      fresh = (await tabIds()).find((id) => !before.includes(id));
      return fresh !== undefined;
    },
    { timeout, message: () => 'no new session tab appeared' },
  );
  return fresh as unknown as string;
}

/** Waits until the tab is gone. */
export async function waitForTabClosed(tabId: string, timeout = TERMINAL_TIMEOUT): Promise<void> {
  await waitForAbsent('titlebar-session-tab', `[data-tab-id="${tabId}"]`, timeout);
}

/** A session tab element by id. */
export function sessionTab(tabId: string) {
  return el('titlebar-session-tab', `[data-tab-id="${tabId}"]`);
}

/** Activates a session tab by clicking it. */
export async function activateTab(tabId: string): Promise<void> {
  await sessionTab(tabId).click();
  await waitForSelectedTab(tabId);
}

/** Activates the Vaults tab and waits for the Hosts or Known Hosts screen. */
export async function activateVaults(): Promise<void> {
  if ((await el('titlebar-tab-vaults').getAttribute('aria-selected')) !== 'true') {
    await el('titlebar-tab-vaults').click();
  }
  await waitForSelectedTab('vaults');
  await el('vaults-tab').waitForDisplayed();
}

/** Closes a session tab with its close button (the tab is activated first). */
export async function closeTab(tabId: string): Promise<void> {
  await activateTab(tabId);
  const close = $(`${tid('titlebar-session-tab', `[data-tab-id="${tabId}"]`)} ${tid('titlebar-session-tab-close')}`);
  await close.waitForExist();
  await close.click();
  await waitForTabClosed(tabId);
}

/** Opens the context menu of a session tab (UI 4.4). */
export async function openTabMenu(tabId: string): Promise<void> {
  await sessionTab(tabId).click({ button: 'right' });
  await waitFor('session-tab-menu');
}

/** Text of a tab's title. */
export async function tabTitle(tabId: string): Promise<string> {
  return (await sessionTabs()).find((tab) => tab.id === tabId)?.title ?? '';
}

// ---------------------------------------------------------------------------
// Panes and sessions
// ---------------------------------------------------------------------------

/** `data-state` of a pane (default: the active pane). */
export async function paneState(paneId?: string): Promise<string | null> {
  const id = paneId ?? (await activePaneId());
  if (id === null) {
    return null;
  }
  return browser.execute(
    (selector: string) => document.querySelector(selector)?.getAttribute('data-state') ?? null,
    tid('terminal-pane', `[data-pane-id="${id}"]`),
  );
}

/** Waits until a pane (default: the active pane, re-resolved while waiting) has this session state. */
export async function waitForPaneState(
  state: string,
  options: { paneId?: string; timeout?: number } = {},
): Promise<string> {
  let current: string | null = null;
  let pane: string | null = null;
  await pollUntil(
    async () => {
      pane = options.paneId ?? (await activePaneId());
      current = pane === null ? null : await paneState(pane);
      return current === state;
    },
    {
      timeout: options.timeout ?? CONNECT_TIMEOUT,
      message: () => `expected pane ${String(pane)} in state "${state}", got "${String(current)}"`,
    },
  );
  return pane as unknown as string;
}

/** The session id shown in a pane. */
export async function paneSessionId(paneId: string): Promise<string | null> {
  return browser.execute(
    (selector: string) => document.querySelector(selector)?.getAttribute('data-session-id') ?? null,
    tid('terminal-pane', `[data-pane-id="${paneId}"]`),
  );
}

/** Selector of an element of the active pane's connection screen. */
async function activePaneElement(id: string, extra = ''): Promise<string> {
  const pane = await waitForActivePane();
  return inPane(pane, id, extra);
}

/** Waits for an element in the active pane (connection screen, prompts, overlays). */
export async function waitInPane(id: string, extra = '', timeout = CONNECT_TIMEOUT) {
  let selector = '';
  await pollUntil(
    async () => {
      selector = await activePaneElement(id, extra);
      return browser.execute((query: string) => document.querySelector(query) !== null, selector);
    },
    { timeout, message: () => `${tid(id, extra)} did not appear in the active pane` },
  );
  return $(selector);
}

/** True when the active pane contains an element with this test id. */
export async function existsInPane(id: string, extra = ''): Promise<boolean> {
  const pane = await activePaneId();
  if (pane === null) {
    return false;
  }
  return browser.execute((query: string) => document.querySelector(query) !== null, inPane(pane, id, extra));
}

/** Waits until the active pane has no element with this test id. */
export async function waitAbsentInPane(id: string, extra = '', timeout = CONNECT_TIMEOUT): Promise<void> {
  await pollUntil(async () => !(await existsInPane(id, extra)), {
    timeout,
    message: () => `${tid(id, extra)} is still in the active pane`,
  });
}

/** Text of an element in the active pane. */
export async function paneText(id: string, extra = ''): Promise<string> {
  const selector = await activePaneElement(id, extra);
  return browser.execute(
    (query: string) => ((document.querySelector(query) as HTMLElement | null)?.innerText ?? '').trim(),
    selector,
  );
}

/** Attribute of an element in the active pane. */
export async function paneAttr(id: string, attribute: string, extra = ''): Promise<string | null> {
  const selector = await activePaneElement(id, extra);
  return browser.execute(
    (query: string, name: string) => document.querySelector(query)?.getAttribute(name) ?? null,
    selector,
    attribute,
  );
}

/** Clicks an element in the active pane. */
export async function clickInPane(id: string, extra = ''): Promise<void> {
  const element = await waitInPane(id, extra);
  await element.click();
}

export type HostKeyAction = 'accept' | 'once' | 'cancel' | 'replace';

/** Answers the host key dialog of the active pane (UI 10.2). */
export async function hostKey(action: HostKeyAction): Promise<void> {
  await waitInPane('host-key-dialog');
  await clickInPane(`host-key-dialog-${action}`);
  await waitAbsentInPane('host-key-dialog', '', 10_000);
}

/** Answers the prompt of the active pane (UI 4.8): types the value, sets "Save", Continue. */
export async function answerPrompt(value: string, options: { save?: boolean; kind?: string } = {}): Promise<void> {
  const extra = options.kind === undefined ? '' : `[data-kind="${options.kind}"]`;
  await waitInPane('session-prompt', extra);
  const input = await waitInPane('session-prompt-input');
  await input.click();
  await browser.keys([Key.Control, 'a']);
  await browser.keys(Key.Backspace);
  if (value !== '') {
    await input.addValue(value);
  }
  if (options.save === true) {
    const save = await waitInPane('session-prompt-save');
    const checked = (await save.getAttribute('aria-checked')) === 'true' || (await save.isSelected());
    if (!checked) {
      await save.click();
    }
  }
  await clickInPane('session-prompt-continue');
}

/** Waits until the active pane is connected and shows the test shell's prompt. */
export async function waitForShell(user: string, options: { paneId?: string; timeout?: number } = {}): Promise<string> {
  const pane = await waitForPaneState('connected', options);
  await waitForTerminalText(shellPrompt(user), { paneId: options.paneId ?? pane, timeout: options.timeout });
  return pane;
}

// ---------------------------------------------------------------------------
// Saved hosts pointing at a test server
// ---------------------------------------------------------------------------

export interface KeyToLink {
  /** OpenSSH private key text (pasted into the dialog). */
  privateKey: string;
  /** Passphrase of an encrypted key. */
  passphrase?: string;
  /** "Save passphrase" (off by default). */
  savePassphrase?: boolean;
  label?: string;
}

/** Opens "+ -> Key" in the open host panel (UI 3.1). */
export async function openKeyDialog(): Promise<void> {
  await (await waitFor('host-panel-ssh-credentials-add')).click();
  await waitFor('host-panel-credentials-menu');
  await (await waitFor('host-panel-credentials-menu-key')).click();
  await waitFor('key-add-dialog');
}

/**
 * Adds a key through "+ -> Key" with the pasted key text and links it to the
 * host of the open panel; answers the passphrase step when the key is
 * encrypted. Returns the linked key's record id.
 */
export async function linkPastedKey(key: KeyToLink): Promise<string> {
  await openKeyDialog();
  if (key.label !== undefined) {
    await clearAndType('key-add-label', key.label);
  }
  const paste = el('key-add-paste');
  await paste.click();
  await paste.addValue(key.privateKey);
  await submitKeyDialog(key);
  return linkedKeyId();
}

/** Clicks Add in the key dialog and answers the passphrase step if it appears. */
export async function submitKeyDialog(key: { passphrase?: string; savePassphrase?: boolean }): Promise<void> {
  await el('key-add-submit').click();
  if (key.passphrase !== undefined) {
    await waitFor('key-add-passphrase', '', 10_000);
    await clearAndType('key-add-passphrase', key.passphrase);
    if (key.savePassphrase === true) {
      const save = el('key-add-save-passphrase');
      const checked = (await save.getAttribute('aria-checked')) === 'true' || (await save.isSelected());
      if (!checked) {
        await save.click();
      }
    }
    await el('key-add-submit').click();
  }
  await waitForAbsent('key-add-dialog', '', 15_000);
}

/** `data-key-id` of the linked key field in the open host panel. */
export async function linkedKeyId(timeout = 10_000): Promise<string> {
  let id: string | null = null;
  await pollUntil(
    async () => {
      id = await browser.execute(
        (selector: string) => document.querySelector(selector)?.getAttribute('data-key-id') ?? null,
        tid('host-panel-ssh-linked-key'),
      );
      return id !== null && id !== '';
    },
    { timeout, message: () => 'host-panel-ssh-linked-key has no data-key-id' },
  );
  return id as unknown as string;
}

export interface SshHostOptions {
  label?: string;
  /** Save the user's password in the host (default true when the user has one). */
  savePassword?: boolean;
  /** A specific password to save instead of the user's (e.g. a wrong one). */
  password?: string;
  /** Username to save (default the user's name; null saves none). */
  username?: string | null;
  /** Parent group. */
  groupId?: string | null;
  /** Key linked through "+ -> Key". */
  key?: KeyToLink;
  /** Terminal theme picked in Show more (`graphite`, `paper`). */
  theme?: string;
  /** Close the panel afterwards (default true). */
  close?: boolean;
}

/** Picks the terminal theme of the open host panel (Show more). */
export async function pickHostTheme(theme: string): Promise<void> {
  const toggle = await waitFor('host-panel-show-more');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
    await toggle.click();
  }
  await (await waitFor('host-panel-theme')).click();
  await waitFor('host-panel-theme-picker');
  await (await waitFor(`host-panel-theme-option-${theme}`)).click();
  await waitForAbsent('host-panel-theme-picker');
}

/**
 * Creates a saved host for a test server user through the host panel
 * (address 127.0.0.1, the server's port, the username and, by default, the
 * password). Returns the record id.
 */
export async function createSshHost(sshd: Sshd, user: SshdUser = sshd.users[0], options: SshHostOptions = {}): Promise<string> {
  const savePassword = options.savePassword ?? user.password !== undefined;
  const password = options.password ?? (savePassword ? user.password : undefined);
  const username = options.username === null ? undefined : (options.username ?? user.name);
  // The Hosts screen of the Vaults tab, whatever tab or section is shown now.
  await activateVaults();
  await ensureHostsScreen();
  const id = await createHost({
    address: sshd.address,
    port: sshd.port,
    ...(options.label === undefined ? {} : { label: options.label }),
    ...(username === undefined ? {} : { username }),
    ...(password === undefined ? {} : { password }),
    ...(options.groupId === undefined ? {} : { groupId: options.groupId }),
    close: false,
  });
  if (options.key !== undefined) {
    await linkPastedKey(options.key);
    await waitForSaved('host-panel', { changed: false });
  }
  if (options.theme !== undefined) {
    await pickHostTheme(options.theme);
    await waitForSaved('host-panel');
  }
  if (options.close !== false) {
    await closePanel('host-panel');
  }
  return id;
}

/** The key fields of a generated key for `linkPastedKey`. */
export function keyToLink(key: GeneratedKey, options: { savePassphrase?: boolean; label?: string } = {}): KeyToLink {
  return {
    privateKey: key.privateKey,
    ...(key.passphrase === undefined ? {} : { passphrase: key.passphrase }),
    ...(options.savePassphrase === undefined ? {} : { savePassphrase: options.savePassphrase }),
    ...(options.label === undefined ? {} : { label: options.label }),
  };
}

// ---------------------------------------------------------------------------
// Connect actions (UI 2.1, 2.4, 3.1, 3.3)
// ---------------------------------------------------------------------------

/** Waits until an element's box is the same in three reads 100 ms apart. */
export async function waitForStablePosition(element: WebdriverIO.Element, timeout = 5_000): Promise<void> {
  const read = () =>
    browser.execute((node: HTMLElement) => {
      const box = node.getBoundingClientRect();
      return `${Math.round(box.x)},${Math.round(box.y)},${Math.round(box.width)},${Math.round(box.height)}`;
    }, element);
  let last = await read();
  let stable = 0;
  const deadline = Date.now() + timeout;
  while (stable < 2 && Date.now() < deadline) {
    await browser.pause(100);
    const current = await read();
    stable = current === last ? stable + 1 : 0;
    last = current;
  }
}

/** Double click with Alt held (UI 2.4: connect in a new window). */
async function altDoubleClick(element: WebdriverIO.Element): Promise<void> {
  await browser.actions([
    browser.action('key').down(Key.Alt).pause(20).pause(20).pause(20).pause(20).pause(20).up(Key.Alt),
    browser
      .action('pointer', { parameters: { pointerType: 'mouse' } })
      .move({ origin: element })
      .down()
      .up()
      .pause(40)
      .down()
      .up(),
  ]);
}

/**
 * Connects to a saved host with a double click on its card (the card must be
 * on the current Hosts screen folder). Returns the new tab's id, or null with
 * `alt` (the session opens in a new window).
 */
export async function connectHost(recordId: string, options: { alt?: boolean } = {}): Promise<string | null> {
  await activateVaults();
  await ensureHostsScreen();
  const card = await hostCard(recordId);
  await card.waitForExist({ timeout: 10_000 });
  await browser.execute((node: HTMLElement) => node.scrollIntoView({ block: 'center' }), card);
  // Select the card first: a single click opens the details panel, which
  // narrows the grid and can move the card (UI 1.3). Then wait until it stays
  // put, so both clicks of the double click land on it. (A plain double click
  // on an unselected card is covered in 2.4-host-card-actions.)
  await waitForStablePosition(card);
  if ((await card.getAttribute('data-selected')) !== 'true' || !(await exists('host-panel', `[data-record-id="${recordId}"]`))) {
    await card.click();
    await el('host-panel', `[data-record-id="${recordId}"]`).waitForExist({ timeout: 10_000 });
  }
  await waitForStablePosition(card);
  const before = await tabIds();
  if (options.alt === true) {
    await altDoubleClick(card);
    return null;
  }
  await card.doubleClick();
  return waitForNewTab(before);
}

/**
 * Connects to a saved host and waits for its shell: double click, the host
 * key dialog (`accept` by default, `none` when the key is known), connected,
 * prompt. Returns the tab id.
 */
export async function openSshSession(
  recordId: string,
  user: string,
  options: { hostKey?: HostKeyAction | 'none' } = {},
): Promise<string> {
  const tab = await connectHost(recordId);
  const action = options.hostKey ?? 'accept';
  if (action !== 'none') {
    await hostKey(action);
  }
  await waitForShell(user);
  return tab as string;
}

/**
 * Types a quick connect text into the Hosts search and presses Enter (Alt+Enter
 * with `alt`). Returns the new tab id (null with `alt`).
 */
export async function quickConnect(text: string, options: { alt?: boolean } = {}): Promise<string | null> {
  await activateVaults();
  await ensureHostsScreen();
  await search(text);
  const button = el('hosts-search-connect');
  await button.waitForEnabled({ timeout: 5_000, timeoutMsg: `the Connect button stays disabled for "${text}"` });
  const before = await tabIds();
  await el('hosts-search-input').click();
  if (options.alt === true) {
    await browser.keys([Key.Alt, Key.Enter]);
    return null;
  }
  await browser.keys(Key.Enter);
  return waitForNewTab(before);
}

/** Selects a host card and opens its panel (from any tab). */
export async function openHostPanel(recordId: string): Promise<void> {
  await activateVaults();
  await ensureHostsScreen();
  await openHost(recordId);
}

/** Switches the Vaults tab to the Hosts screen when another section (Known Hosts) is shown. */
export async function ensureHostsScreen(): Promise<void> {
  if (!(await exists('hosts-screen'))) {
    await (await waitFor('sidebar-hosts')).click();
    await waitForHostsReady();
  }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/** Waits for a window handle that is not in `before` and returns it. */
export async function waitForNewWindow(before: string[], timeout = 20_000): Promise<string> {
  let handle: string | undefined;
  await pollUntil(
    async () => {
      handle = (await browser.getWindowHandles()).find((candidate) => !before.includes(candidate));
      return handle !== undefined;
    },
    { timeout, message: () => 'no new window opened' },
  );
  return handle as unknown as string;
}

/** Switches to the window whose `app-root[data-window-label]` is `label`. */
export async function switchToWindowLabel(label: string, timeout = 20_000): Promise<void> {
  await pollUntil(
    async () => {
      for (const handle of await browser.getWindowHandles()) {
        await browser.switchToWindow(handle);
        const current = await browser.execute(
          (selector: string) => document.querySelector(selector)?.getAttribute('data-window-label') ?? null,
          tid('app-root'),
        );
        if (current === label) {
          return true;
        }
      }
      return false;
    },
    { timeout, message: () => `window ${label} not found` },
  );
}

/**
 * Closes the current window with the title bar button, confirming the
 * window close dialog when sessions are active, and switches to `next`.
 */
export async function closeWindowConfirmed(next: string): Promise<void> {
  const before = (await browser.getWindowHandles()).length;
  await el('titlebar-window-close').click();
  await pollUntil(
    async () => {
      if ((await browser.getWindowHandles()).length < before) {
        return true;
      }
      if (await exists('window-close-confirm-dialog-confirm')) {
        await el('window-close-confirm-dialog-confirm').click();
      }
      return false;
    },
    { timeout: 15_000, message: () => 'the window did not close' },
  );
  await browser.switchToWindow(next);
}

// ---------------------------------------------------------------------------
// Known hosts (UI 10.1, 10.2)
// ---------------------------------------------------------------------------

/**
 * A hashed known_hosts host (`|1|<salt>|<hash>`, OpenSSH hashing): HMAC-SHA1
 * with a random 20-byte salt over `host` (port 22) or `[host]:port`.
 */
export function hashedHost(address: string, port = 22): string {
  const name = port === 22 ? address : `[${address}]:${port}`;
  const salt = crypto.randomBytes(20);
  const hash = crypto.createHmac('sha1', salt).update(name).digest();
  return `|1|${salt.toString('base64')}|${hash.toString('base64')}`;
}

/** Seeds a known host entry through the e2e mock action `known_host_add` (source `import`). */
export async function seedKnownHost(entry: KnownHostAddInput): Promise<void> {
  await mock({ kind: 'known_host_add', ...entry });
}

/** Opens the Known Hosts screen from the sidebar (UI 1.2, 10.1). */
export async function openKnownHosts(): Promise<void> {
  await activateVaults();
  await (await waitFor('sidebar-known-hosts')).click();
  await el('known-hosts-screen', '[data-ready="true"]').waitForExist({ timeout: 10_000 });
}

/** Opens the Hosts screen from the sidebar. */
export async function openHostsScreen(): Promise<void> {
  await activateVaults();
  await (await waitFor('sidebar-hosts')).click();
  await waitForHostsReady();
}

/** Record ids of the known host cards in display order. */
export async function knownHostIds(): Promise<string[]> {
  return (await attrOfAll('known-host-card', 'data-record-id')).filter((id): id is string => id !== null);
}

/** Known host card by record id. */
export function knownHostCard(id: string) {
  return el('known-host-card', `[data-record-id="${id}"]`);
}

/** Text of a part of a known host card. */
export async function knownHostCardText(id: string, part: string): Promise<string> {
  return browser.execute(
    (query: string) => ((document.querySelector(query) as HTMLElement | null)?.innerText ?? '').trim(),
    `${tid('known-host-card', `[data-record-id="${id}"]`)} ${tid(part)}`,
  );
}

/** Waits until the number of known host cards is `n`; returns their ids. */
export async function waitForKnownHostCount(n: number, timeout = 10_000): Promise<string[]> {
  let ids: string[] = [];
  await pollUntil(
    async () => {
      ids = await knownHostIds();
      return ids.length === n;
    },
    { timeout, message: () => `expected ${n} known host cards, got ${ids.length}` },
  );
  return ids;
}

// ---------------------------------------------------------------------------
// New tab page (UI 6)
// ---------------------------------------------------------------------------

/** CSS selector of an element with a test id inside the new tab page of a tab. */
export function inNewTab(tabId: string, id: string, extra = ''): string {
  return `${tid('newtab-page', `[data-tab-id="${tabId}"]`)} ${tid(id, extra)}`;
}

/** Opens a new tab with "+" and waits for its page; returns the tab id. */
export async function openNewTabPage(): Promise<string> {
  const before = await tabIds();
  await (await waitFor('titlebar-new-tab')).click();
  const tab = await waitForNewTab(before);
  await waitForSelectedTab(tab);
  await $(tid('newtab-page', `[data-tab-id="${tab}"]`)).waitForDisplayed({ timeout: 10_000 });
  return tab;
}

/** Test ids and identities of the HostPicker results in display order (`P-result-tab`, `P-result-host`). */
export async function pickerResults(scope: string, prefix: string): Promise<{ kind: 'tab' | 'host'; id: string }[]> {
  return browser.execute(
    (root: string, tabId: string, hostId: string) => {
      const container = document.querySelector(root);
      if (!container) {
        return [];
      }
      return Array.from(container.querySelectorAll(`[data-testid="${tabId}"], [data-testid="${hostId}"]`)).map((node) =>
        node.getAttribute('data-testid') === tabId
          ? { kind: 'tab' as const, id: node.getAttribute('data-tab-id') ?? '' }
          : { kind: 'host' as const, id: node.getAttribute('data-host-id') ?? '' },
      );
    },
    scope,
    `${prefix}-result-tab`,
    `${prefix}-result-host`,
  );
}
