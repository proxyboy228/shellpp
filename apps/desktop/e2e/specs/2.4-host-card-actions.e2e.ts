// UI 2.4 Host card actions: click selects and opens details, E edits (focus
// in Address), arrow keys move the selection, the context menu (Quick
// Connect, Connect > via SSH with the "new window" hint, Edit Host Details,
// Move to, Duplicate, Copy Link with info, Remove with undo). Stage 2: double
// click and Enter connect (new tab), Alt + double click opens the session in
// a new window, Connect acts on every selected host. Via SFTP (stage 4), Add
// to Workspace (stage 9), Collaborate and Copy to (stage 8) are hidden.
// Connections go to the test SSH server only (hosts in the "servers" group).

import { Key } from 'webdriverio';
import {
  activateVaults,
  cardText,
  clipboardText,
  closePanel,
  closeWindowConfirmed,
  connectHost,
  createGroup,
  createHost,
  createSshHost,
  ctrlClick,
  el,
  expectAbsent,
  expectPresent,
  hostKey,
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
  openSubmenu,
  pressEscape,
  runCommand,
  selectedHostIds,
  startFresh,
  tabIds,
  tabTitle,
  textAt,
  tid,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForBoot,
  waitForHostIds,
  waitForLockState,
  waitForNewTab,
  waitForNewWindow,
  waitForSelectedTab,
  waitForShell,
  waitForStablePosition,
  waitForTabClosed,
  waitForTabCount,
  waitForToast,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

const MENU_ITEMS = [
  'host-menu-quick-connect',
  'host-menu-connect',
  'host-menu-edit',
  'host-menu-move-to',
  'host-menu-duplicate',
  'host-menu-copy-link',
  'host-menu-remove',
];

const HIDDEN_ITEMS = [
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

  it('lists the context menu entries in order, with the Connect submenu', async () => {
    await openHostMenu(alpha);
    expect(await menuItemIds('host-context-menu')).toEqual(MENU_ITEMS);
    // The shortcut hint (WebDriver getText is empty while the menu fades in).
    await browser.waitUntil(async () => (await textAt(tid('host-menu-quick-connect'))).includes('Enter'), {
      timeout: 5_000,
      timeoutMsg: 'Quick Connect does not show the Enter hint',
    });
    await openSubmenu('host-menu-connect');
    await browser.waitUntil(async () => (await menuItemIds('host-menu-connect-menu')).includes('host-menu-connect-ssh'), {
      timeout: 5_000,
      timeoutMsg: 'the Connect submenu does not list "via SSH"',
    });
    await expectPresent(['host-menu-connect-ssh', 'host-menu-connect-alt-hint']);
    await expectAbsent(HIDDEN_ITEMS);
    await pressEscape();
    if ((await browser.execute(() => document.querySelector('[data-testid="host-context-menu"]') !== null)) === true) {
      await pressEscape();
    }
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

  describe('connect actions', () => {
    let sshd: Sshd;
    let user: SshdUser;
    let servers: string;
    let srv1: string;
    let srv2: string;
    let srv3: string;

    /** Waits for the shell in the active tab, checks its title and ends it with `exit`. */
    async function finishSession(tab: string, title: string): Promise<void> {
      await waitForSelectedTab(tab);
      await waitForShell(user.name);
      expect(await tabTitle(tab)).toBe(title);
      await runCommand('exit');
      await waitForTabClosed(tab);
    }

    before(async () => {
      sshd = await startSshd();
      user = sshd.users[0];
      servers = await createGroup({ name: 'servers' });
      srv1 = await createSshHost(sshd, user, { label: 'srv-1', groupId: servers });
      srv2 = await createSshHost(sshd, user, { label: 'srv-2', groupId: servers });
      srv3 = await createSshHost(sshd, user, { label: 'srv-3', groupId: servers });
      await openFolder(servers);
      await waitForHostIds([srv1, srv2, srv3], true);
    });

    after(async () => {
      await stopAllSshd();
    });

    it('connects in a new session tab on a double click, wherever the card is', async () => {
      // A plain double click on a card that is not selected: its first click
      // opens the details panel, which narrows the grid (UI 1.3); the third
      // card of a row then moves, and the connection must still start.
      await expectAbsent(['host-panel']);
      const card = await hostCard(srv3);
      await waitForStablePosition(card);
      const before = await tabIds();
      await card.doubleClick();
      const tab = await waitForNewTab(before);
      await hostKey('accept');
      await finishSession(tab, 'srv-3');
    });

    it('connects the selected card on a double click', async () => {
      const tab = await connectHost(srv1);
      await finishSession(tab as string, 'srv-1');
    });

    it('connects the selected card on Enter', async () => {
      await activateVaults();
      await hostCard(srv1).click();
      await waitFor('host-panel', `[data-record-id="${srv1}"]`);
      await pressEscape();
      await waitForAbsent('host-panel');
      expect(await selectedHostIds()).toEqual([srv1]);
      const before = await tabIds();
      await browser.keys(Key.Enter);
      await finishSession(await waitForNewTab(before), 'srv-1');
    });

    it('connects with Quick Connect and with Connect > via SSH', async () => {
      await activateVaults();
      await openHostMenu(srv2);
      let before = await tabIds();
      await el('host-menu-quick-connect').click();
      await finishSession(await waitForNewTab(before), 'srv-2');

      await activateVaults();
      await openHostMenu(srv2);
      before = await tabIds();
      await menuPath('host-menu-connect', 'host-menu-connect-ssh');
      await finishSession(await waitForNewTab(before), 'srv-2');
    });

    it('opens the session in a new window on Alt + double click', async () => {
      await activateVaults();
      const main = await browser.getWindowHandle();
      const handles = await browser.getWindowHandles();
      expect(await connectHost(srv1, { alt: true })).toBeNull();
      const second = await waitForNewWindow(handles);
      await browser.switchToWindow(second);
      await waitForBoot();
      await waitForLockState('unlocked');
      const [tab] = await waitForTabCount(1);
      await waitForSelectedTab(tab);
      await waitForShell(user.name);
      expect(await tabTitle(tab)).toBe('srv-1');
      await closeWindowConfirmed(main);
      expect(await tabIds()).toEqual([]);
    });

    it('connects every selected host with Connect, one tab each', async () => {
      await activateVaults();
      await hostCard(srv1).click();
      await ctrlClick(hostCard(srv2));
      await browser.waitUntil(async () => (await selectedHostIds()).length === 2, {
        timeout: 5_000,
        timeoutMsg: 'the two hosts were not selected',
      });
      await openHostMenu(srv2);
      const before = await tabIds();
      await menuPath('host-menu-connect', 'host-menu-connect-ssh');
      const after = await waitForTabCount(before.length + 2);
      const opened = after.filter((id) => !before.includes(id));
      const titles = await Promise.all(opened.map((id) => tabTitle(id)));
      expect([...titles].sort()).toEqual(['srv-1', 'srv-2']);
      for (const [index, tab] of opened.entries()) {
        await el('titlebar-session-tab', `[data-tab-id="${tab}"]`).click();
        await finishSession(tab, titles[index]);
      }
    });
  });
});
