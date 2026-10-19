// Hosts screen (UI 2), host panel (UI 3.1-3.3) and group panel (UI 3.4)
// helpers. Records are created through the UI and identified by the
// `data-record-id` the panel reports once the draft became a record.

import { Key } from 'webdriverio';
import { clearAndType, waitForHostsReady } from './app.ts';
import { rightClick } from './pointer.ts';
import { attrOfAll, count, el, tid, tidWith, waitFor, waitForAbsent } from './selectors.ts';

export type PanelId = 'host-panel' | 'group-panel';

/** `data-save-state` of a panel: `draft`, `pending`, `saving`, `saved` or `error`. */
export async function saveState(panel: PanelId): Promise<string | null> {
  return browser.execute(
    (selector: string) => document.querySelector(selector)?.getAttribute('data-save-state') ?? null,
    tid(panel),
  );
}

/** `data-record-id` of a panel (null for a draft). */
export async function panelRecordId(panel: PanelId): Promise<string | null> {
  return browser.execute(
    (selector: string) => document.querySelector(selector)?.getAttribute('data-record-id') ?? null,
    tid(panel),
  );
}

/** Waits until the panel's draft became a record and returns its id. */
export async function waitForRecordId(panel: PanelId, timeout = 10_000): Promise<string> {
  let id: string | null = null;
  await browser.waitUntil(
    async () => {
      id = await panelRecordId(panel);
      return id !== null && id !== '';
    },
    { timeout, timeoutMsg: `${panel} did not get a data-record-id` },
  );
  return id as unknown as string;
}

/**
 * Waits for the autosave of the last change (500 ms debounce): first for the
 * panel to leave `saved` (it may already have), then for `saved` again.
 * `changed: false` skips the first step when no edit is pending.
 */
export async function waitForSaved(
  panel: PanelId = 'host-panel',
  { timeout = 10_000, changed = true }: { timeout?: number; changed?: boolean } = {},
): Promise<void> {
  if (changed) {
    try {
      await browser.waitUntil(async () => (await saveState(panel)) !== 'saved', { timeout: 800, interval: 25 });
    } catch {
      // Already saved before we looked, or nothing changed.
    }
  }
  await browser.waitUntil(async () => (await saveState(panel)) === 'saved', {
    timeout,
    interval: 50,
    timeoutMsg: `${panel} did not reach data-save-state="saved" (now "${String(await saveState(panel))}")`,
  });
}

/** Closes a details panel with its collapse button. */
export async function closePanel(panel: PanelId): Promise<void> {
  if ((await count(panel)) === 0) {
    return;
  }
  await el(`${panel}-collapse`).click();
  await waitForAbsent(panel);
}

/** Types into a panel field (clearing it first). */
export async function setField(id: string, value: string): Promise<void> {
  await clearAndType(id, value);
}

/** Opens the New host panel (UI 2.2). */
export async function openNewHostPanel(): Promise<void> {
  await (await waitFor('hosts-new-host-main')).click();
  await waitFor('host-panel');
}

/** Opens the New Group panel from the split button menu (UI 2.2). */
export async function openNewGroupPanel(): Promise<void> {
  await (await waitFor('hosts-new-host-toggle')).click();
  await (await waitFor('hosts-new-host-item-new-group')).click();
  await waitFor('group-panel');
}

/**
 * Picks a group in a Parent Group picker (`host-panel-group` or
 * `group-panel-parent`); null picks the top level.
 */
export async function pickGroup(picker: 'host-panel-group' | 'group-panel-parent', groupId: string | null): Promise<void> {
  await el(picker).click();
  await waitFor(`${picker}-popover`);
  const option = el(`${picker}-option`, `[data-group-id="${groupId ?? ''}"]`);
  await option.waitForExist();
  await option.click();
  await waitForAbsent(`${picker}-popover`);
}

/** Adds a tag in the host panel with Enter. */
export async function addTag(tag: string): Promise<void> {
  const input = el('host-panel-tags-input');
  await input.click();
  await input.addValue(tag);
  await browser.keys(Key.Enter);
  await el('host-panel-tag', `[data-tag="${tag}"]`).waitForExist();
}

export interface HostInput {
  address: string;
  label?: string;
  username?: string;
  port?: string | number;
  password?: string;
  tags?: string[];
  /** Parent group picked in the panel (undefined keeps the default). */
  groupId?: string | null;
  /** Close the panel afterwards (default true). */
  close?: boolean;
}

/** Creates a host through the host panel; returns its record id. */
export async function createHost(input: HostInput): Promise<string> {
  await openNewHostPanel();
  const address = el('host-panel-address');
  await address.waitForExist();
  await address.click();
  await address.addValue(input.address);
  const id = await waitForRecordId('host-panel');
  await waitForSaved('host-panel', { changed: false });
  if (input.label !== undefined) {
    await setField('host-panel-label', input.label);
  }
  if (input.username !== undefined) {
    await setField('host-panel-ssh-username', input.username);
  }
  if (input.port !== undefined) {
    await setField('host-panel-ssh-port', String(input.port));
  }
  if (input.password !== undefined) {
    await setField('host-panel-ssh-password', input.password);
  }
  for (const tag of input.tags ?? []) {
    await addTag(tag);
  }
  if (input.groupId !== undefined) {
    await pickGroup('host-panel-group', input.groupId);
  }
  const edited =
    input.label !== undefined ||
    input.username !== undefined ||
    input.port !== undefined ||
    input.password !== undefined ||
    (input.tags ?? []).length > 0 ||
    input.groupId !== undefined;
  await waitForSaved('host-panel', { changed: edited });
  if (input.close !== false) {
    await closePanel('host-panel');
  }
  return id;
}

export interface GroupInput {
  name: string;
  /** Parent group picked in the panel (undefined keeps the current folder). */
  parentId?: string | null;
  username?: string;
  port?: string | number;
  close?: boolean;
}

/** Creates a group through New Group and the group panel; returns its record id. */
export async function createGroup(input: GroupInput): Promise<string> {
  await openNewGroupPanel();
  const name = el('group-panel-name');
  await name.waitForExist();
  await name.click();
  await name.addValue(input.name);
  const id = await waitForRecordId('group-panel');
  await waitForSaved('group-panel', { changed: false });
  if (input.parentId !== undefined) {
    await pickGroup('group-panel-parent', input.parentId);
  }
  if (input.username !== undefined) {
    await setField('group-panel-ssh-username', input.username);
  }
  if (input.port !== undefined) {
    await setField('group-panel-ssh-port', String(input.port));
  }
  const edited = input.parentId !== undefined || input.username !== undefined || input.port !== undefined;
  await waitForSaved('group-panel', { changed: edited });
  if (input.close !== false) {
    await closePanel('group-panel');
  }
  return id;
}

/** Host card by record id. */
export function hostCard(id: string) {
  return el('host-card', `[data-record-id="${id}"]`);
}

/** Group card by record id. */
export function groupCard(id: string) {
  return el('group-card', `[data-record-id="${id}"]`);
}

/** Record ids of the host cards in display order. */
export async function hostIds(): Promise<string[]> {
  return (await attrOfAll('host-card', 'data-record-id')).filter((id): id is string => id !== null);
}

/** Record ids of the group cards in display order. */
export async function groupIds(): Promise<string[]> {
  return (await attrOfAll('group-card', 'data-record-id')).filter((id): id is string => id !== null);
}

/** Record ids of the selected host cards in display order. */
export async function selectedHostIds(): Promise<string[]> {
  return (await attrOfAll('host-card', 'data-record-id', '[data-selected="true"]')).filter(
    (id): id is string => id !== null,
  );
}

function sameMembers(actual: string[], expected: string[]): boolean {
  return actual.length === expected.length && [...actual].sort().join() === [...expected].sort().join();
}

/** Waits until the visible host cards are exactly these ids (in this order when `ordered`). */
export async function waitForHostIds(expected: string[], ordered = false, timeout = 10_000): Promise<void> {
  let actual: string[] = [];
  await browser.waitUntil(
    async () => {
      actual = await hostIds();
      return ordered ? actual.join() === expected.join() : sameMembers(actual, expected);
    },
    { timeout, timeoutMsg: `host cards ${JSON.stringify(actual)} != ${JSON.stringify(expected)}` },
  );
}

/** Waits until the visible group cards are exactly these ids (in this order when `ordered`). */
export async function waitForGroupIds(expected: string[], ordered = false, timeout = 10_000): Promise<void> {
  let actual: string[] = [];
  await browser.waitUntil(
    async () => {
      actual = await groupIds();
      return ordered ? actual.join() === expected.join() : sameMembers(actual, expected);
    },
    { timeout, timeoutMsg: `group cards ${JSON.stringify(actual)} != ${JSON.stringify(expected)}` },
  );
}

/** Text of a sub element of a card (`host-card-title`, `group-card-count`, ...). */
export async function cardText(card: 'host-card' | 'group-card', id: string, part: string): Promise<string> {
  const selector = `${tidWith(card, 'record-id', id)} ${tid(part)}`;
  // The card may still be loading (for example right after a folder change).
  await browser.waitUntil(async () => browser.execute((query: string) => document.querySelector(query) !== null, selector), {
    timeout: 5_000,
    timeoutMsg: `${selector} did not appear`,
  });
  return browser.execute(
    (query: string) => (document.querySelector(query) as HTMLElement | null)?.innerText.trim() ?? '',
    selector,
  );
}

/** Opens a group folder by clicking its card and waits for the breadcrumb. */
export async function openFolder(groupId: string): Promise<void> {
  await groupCard(groupId).click();
  await el('hosts-breadcrumb', `[data-group-id="${groupId}"][aria-current]`).waitForExist();
  await waitForHostsReady();
}

/** Returns to the top level with the root breadcrumb. */
export async function goToRoot(): Promise<void> {
  await (await waitFor('hosts-breadcrumb-root')).click();
  await browser.waitUntil(async () => (await count('hosts-breadcrumb')) === 0, {
    timeoutMsg: 'breadcrumbs did not return to the root',
  });
}

/** Group ids of the breadcrumbs (root excluded), outermost first. */
export async function breadcrumbIds(): Promise<string[]> {
  return (await attrOfAll('hosts-breadcrumb', 'data-group-id')).filter((id): id is string => id !== null);
}

/** Types into the search bar (UI 2.1). */
export async function search(text: string): Promise<void> {
  await clearAndType('hosts-search-input', text);
}

/** Clears the search with its clear button. */
export async function clearSearch(): Promise<void> {
  await el('hosts-search-clear').click();
  await browser.waitUntil(
    async () =>
      (await browser.execute(
        (selector: string) => (document.querySelector(selector) as HTMLInputElement | null)?.value ?? '',
        tid('hosts-search-input'),
      )) === '',
  );
}

/** Opens the context menu of a host card. */
export async function openHostMenu(id: string): Promise<void> {
  await rightClick(hostCard(id));
  await waitFor('host-context-menu');
}

/** Opens the context menu of a group card. */
export async function openGroupMenu(id: string): Promise<void> {
  await rightClick(groupCard(id));
  await waitFor('group-context-menu');
}

/** Opens the context menu of a host or group card (plan name; see `openHostMenu`, `openGroupMenu`). */
export async function openContextMenu(card: 'host-card' | 'group-card', id: string): Promise<void> {
  await (card === 'host-card' ? openHostMenu(id) : openGroupMenu(id));
}

/** Opens the host panel of a host by clicking its card. */
export async function openHost(id: string): Promise<void> {
  await hostCard(id).click();
  await el('host-panel', `[data-record-id="${id}"]`).waitForExist();
}

/** Opens the group panel of a group through its context menu (Edit). */
export async function editGroup(id: string): Promise<void> {
  await openGroupMenu(id);
  await el('group-menu-edit').click();
  await el('group-panel', `[data-record-id="${id}"]`).waitForExist();
}

/** Sets a sort mode through the sort menu (UI 2.2). */
export async function setSort(mode: 'manual' | 'name' | 'created' | 'modified'): Promise<void> {
  await (await waitFor('hosts-sort-button')).click();
  await waitFor('hosts-sort-menu');
  await el(`hosts-sort-${mode}`).click();
  await el('hosts-screen', `[data-sort="${mode}"]`).waitForExist();
}

/** Waits for a toast with this test id. */
export async function waitForToast(id: string, timeout = 10_000) {
  return waitFor(id, '', timeout);
}
