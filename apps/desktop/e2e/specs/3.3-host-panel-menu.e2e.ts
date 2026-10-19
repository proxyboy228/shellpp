// UI 3.3 "..." menu in the host panel header: Connect (stage 2), Duplicate
// and Remove; Add Telnet (stage 9) is hidden. Connect goes to the test SSH
// server only.

import {
  cardText,
  createHost,
  createSshHost,
  el,
  expectAbsent,
  hostIds,
  hostKey,
  menuItemIds,
  openHost,
  pressEscape,
  runCommand,
  startFresh,
  tabIds,
  tabTitle,
  waitFor,
  waitForAbsent,
  waitForNewTab,
  waitForSelectedTab,
  waitForShell,
  waitForTabClosed,
  waitForToast,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd } from '../support/sshd.ts';

async function openPanelMenu(): Promise<void> {
  await el('host-panel-menu-button').click();
  await waitFor('host-panel-menu');
}

describe('UI 3.3 Host panel menu', () => {
  let sshd: Sshd;
  let hostId: string;
  let serverHost: string;

  before(async () => {
    await startFresh();
    hostId = await createHost({ address: 'menu.example.com', label: 'Menu host' });
    sshd = await startSshd();
    serverHost = await createSshHost(sshd, sshd.users[0], { label: 'Menu server' });
  });

  after(async () => {
    await stopAllSshd();
  });

  it('lists Connect, Duplicate and Remove, without the later-stage entries', async () => {
    await openHost(hostId);
    await openPanelMenu();
    expect(await menuItemIds('host-panel-menu')).toEqual([
      'host-panel-menu-connect',
      'host-panel-menu-duplicate',
      'host-panel-menu-remove',
    ]);
    await expectAbsent(['host-panel-menu-add-telnet']);
    await pressEscape();
    await waitForAbsent('host-panel-menu');
  });

  it('connects to the host with Connect', async () => {
    await openHost(serverHost);
    await openPanelMenu();
    const before = await tabIds();
    await el('host-panel-menu-connect').click();
    const tab = await waitForNewTab(before);
    await waitForSelectedTab(tab);
    await hostKey('accept');
    await waitForShell(sshd.users[0].name);
    expect(await tabTitle(tab)).toBe('Menu server');
    await runCommand('exit');
    await waitForTabClosed(tab);
  });

  it('duplicates the host with " Copy" appended', async () => {
    await openHost(hostId);
    const before = await hostIds();
    await openPanelMenu();
    await el('host-panel-menu-duplicate').click();
    await browser.waitUntil(async () => (await hostIds()).length === before.length + 1);
    const copy = (await hostIds()).find((id) => !before.includes(id)) as string;
    expect(await cardText('host-card', copy, 'host-card-title')).toBe('Menu host Copy');
  });

  it('removes the host after confirmation and closes the panel', async () => {
    await openHost(hostId);
    await openPanelMenu();
    await el('host-panel-menu-remove').click();
    await waitFor('host-remove-dialog');
    await el('host-remove-dialog-confirm').click();
    await waitForAbsent('host-card', `[data-record-id="${hostId}"]`);
    await waitForAbsent('host-panel');
    await waitForToast('hosts-undo-toast');
  });
});
