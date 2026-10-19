// UI 2.4 Host card actions: click selects and opens details, E edits (focus
// in Address), arrow keys move the selection, the context menu (Edit Host
// Details, Move to, Duplicate, Copy Link with info, Remove with undo).
// Connect actions belong to stage 2 and later (hidden).

import { Key } from 'webdriverio';
import {
  cardText,
  clipboardText,
  closePanel,
  createGroup,
  createHost,
  ctrlClick,
  el,
  expectAbsent,
  goToRoot,
  groupCard,
  hostCard,
  hostIds,
  isFocused,
  menuItemIds,
  menuPath,
  newPassword,
  openFolder,
  openHost,
  openHostMenu,
  pressEscape,
  selectedHostIds,
  startFresh,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForHostIds,
  waitForToast,
} from '../helpers/index.ts';

const MENU_ITEMS = ['host-menu-edit', 'host-menu-move-to', 'host-menu-duplicate', 'host-menu-copy-link', 'host-menu-remove'];

const HIDDEN_ITEMS = [
  'host-menu-quick-connect', // stage 2
  'host-menu-connect', // stage 2
  'host-menu-connect-ssh', // stage 2
  'host-menu-connect-sftp', // stage 4
  'host-menu-add-to-workspace', // stage 9
  'host-menu-collaborate', // stage 8
  'host-menu-copy-to', // stage 8
];

describe('UI 2.4 Host card actions', () => {
  const secret = newPassword();
  let targets: string;
  let alpha: string;
  let bravo: string;
  let charlie: string;

  before(async () => {
    await startFresh();
    targets = await createGroup({ name: 'targets' });
    alpha = await createHost({ address: 'alpha.example.com', label: 'alpha', password: secret });
    bravo = await createHost({ address: 'bravo.example.com', label: 'bravo' });
    charlie = await createHost({ address: 'charlie.example.com', label: 'charlie' });
    await waitForHostIds([alpha, bravo, charlie], true);
  });

  it('selects the card and shows its details on a single click', async () => {
    await hostCard(alpha).click();
    await expect(hostCard(alpha)).toHaveAttribute('data-selected', 'true');
    await waitFor('host-panel', `[data-record-id="${alpha}"]`);
  });

  it('treats a double click like a single click in stage 1 (connect is stage 2)', async () => {
    await hostCard(bravo).doubleClick();
    await waitFor('host-panel', `[data-record-id="${bravo}"]`);
    expect(await selectedHostIds()).toEqual([bravo]);
  });

  it('opens the details with the focus in Address on E', async () => {
    await hostCard(alpha).click();
    await waitFor('host-panel', `[data-record-id="${alpha}"]`);
    await pressEscape();
    await waitForAbsent('host-panel');
    expect(await selectedHostIds()).toEqual([alpha]);
    await browser.keys('e');
    await waitFor('host-panel', `[data-record-id="${alpha}"]`);
    await browser.waitUntil(() => isFocused('host-panel-address'), { timeout: 5_000 });
    await closePanel('host-panel');
  });

  it('moves the selection with the arrow keys', async () => {
    await hostCard(alpha).click();
    await browser.keys(Key.ArrowRight);
    await browser.waitUntil(async () => (await selectedHostIds()).join() === bravo, {
      timeoutMsg: 'ArrowRight did not select the next card',
    });
    await browser.keys(Key.ArrowRight);
    await browser.waitUntil(async () => (await selectedHostIds()).join() === charlie);
    await browser.keys(Key.ArrowLeft);
    await browser.waitUntil(async () => (await selectedHostIds()).join() === bravo);
    await closePanel('host-panel');
  });

  it('lists the stage-1 context menu entries in order', async () => {
    await openHostMenu(alpha);
    expect(await menuItemIds('host-context-menu')).toEqual(MENU_ITEMS);
    await expectAbsent(HIDDEN_ITEMS);
    await pressEscape();
    await waitForAbsent('host-context-menu');
  });

  it('opens the details with the focus in Address from Edit Host Details', async () => {
    await openHostMenu(bravo);
    await el('host-menu-edit').click();
    await waitFor('host-panel', `[data-record-id="${bravo}"]`);
    await browser.waitUntil(() => isFocused('host-panel-address'), { timeout: 5_000 });
    await closePanel('host-panel');
  });

  it('moves a host into a group and back to the top level with Move to', async () => {
    await openHostMenu(bravo);
    await menuPath('host-menu-move-to', `host-menu-move-to-group-${targets}`);
    await waitForHostIds([alpha, charlie], true);
    await browser.waitUntil(async () => (await cardText('group-card', targets, 'group-card-count')).includes('1'));

    await openFolder(targets);
    await waitForHostIds([bravo]);
    await openHostMenu(bravo);
    await menuPath('host-menu-move-to', 'host-menu-move-to-top-level');
    await waitForHostIds([]);
    await goToRoot();
    await waitForHostIds([alpha, charlie, bravo]);
    await expect(groupCard(targets)).toBeExisting();
  });

  it('duplicates a host right after the original with " Copy" appended', async () => {
    const before = await hostIds();
    await openHostMenu(alpha);
    await el('host-menu-duplicate').click();
    await browser.waitUntil(async () => (await hostIds()).length === before.length + 1);
    const after = await hostIds();
    const copy = after.find((id) => !before.includes(id)) as string;
    expect(after.indexOf(copy)).toBe(after.indexOf(alpha) + 1);
    expect(await cardText('host-card', copy, 'host-card-title')).toBe('alpha Copy');
  });

  it('copies the host link and explains who can open it', async () => {
    await openHostMenu(alpha);
    const info = el('host-menu-copy-link-info');
    await info.moveTo();
    const explanation = await browser.execute((selector: string) => {
      const icon = document.querySelector(selector) as HTMLElement | null;
      if (!icon) {
        return '';
      }
      const describedBy = icon.getAttribute('aria-describedby');
      const described = describedBy ? (document.getElementById(describedBy)?.textContent ?? '') : '';
      return [icon.getAttribute('aria-label'), icon.getAttribute('title'), described, document.querySelector('[role="tooltip"]')?.textContent]
        .filter(Boolean)
        .join(' ');
    }, '[data-testid="host-menu-copy-link-info"]');
    expect(explanation.trim().length).toBeGreaterThan(10);
    await el('host-menu-copy-link').click();
    await waitForToast('hosts-link-copied-toast');
    expect(await clipboardText()).toBe(`shellpp://host/${alpha}`);
  });

  it('keeps the host when Remove is cancelled', async () => {
    await openHostMenu(charlie);
    await el('host-menu-remove').click();
    await waitFor('host-remove-dialog');
    await el('host-remove-dialog-cancel').click();
    await waitForAbsent('host-remove-dialog');
    await expect(hostCard(charlie)).toBeExisting();
  });

  it('removes a host after confirmation and restores it with Undo, password included', async () => {
    await openHostMenu(alpha);
    await el('host-menu-remove').click();
    const dialog = await waitFor('host-remove-dialog');
    await expect(dialog).toHaveAttribute('data-count', '1');
    await el('host-remove-dialog-confirm').click();
    await waitForAbsent('host-card', `[data-record-id="${alpha}"]`);
    await waitForToast('hosts-undo-toast');
    await el('hosts-undo-toast-action').click();
    await waitFor('host-card', `[data-record-id="${alpha}"]`);

    await openHost(alpha);
    await el('host-panel-ssh-password-toggle').click();
    await browser.waitUntil(async () => (await valueOf('host-panel-ssh-password')) === secret, {
      timeout: 10_000,
      timeoutMsg: 'the restored host does not reveal its password',
    });
    await closePanel('host-panel');
  });

  it('removes every selected host at once', async () => {
    await hostCard(bravo).click();
    await ctrlClick(hostCard(charlie));
    await browser.waitUntil(async () => (await selectedHostIds()).length === 2);
    await openHostMenu(charlie);
    await el('host-menu-remove').click();
    const dialog = await waitFor('host-remove-dialog');
    await expect(dialog).toHaveAttribute('data-count', '2');
    await el('host-remove-dialog-confirm').click();
    await browser.waitUntil(async () => {
      const ids = await hostIds();
      return !ids.includes(bravo) && !ids.includes(charlie);
    });
    expect(await hostIds()).toContain(alpha);
  });
});
