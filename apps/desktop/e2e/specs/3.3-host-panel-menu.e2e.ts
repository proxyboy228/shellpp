// UI 3.3 "..." menu in the host panel header: Duplicate and Remove in stage
// 1; Connect (stage 2) and Add Telnet (stage 9) are hidden.

import {
  cardText,
  createHost,
  el,
  expectAbsent,
  hostIds,
  menuItemIds,
  openHost,
  pressEscape,
  startFresh,
  waitFor,
  waitForAbsent,
  waitForToast,
} from '../helpers/index.ts';

async function openPanelMenu(): Promise<void> {
  await el('host-panel-menu-button').click();
  await waitFor('host-panel-menu');
}

describe('UI 3.3 Host panel menu', () => {
  let hostId: string;

  before(async () => {
    await startFresh();
    hostId = await createHost({ address: 'menu.example.com', label: 'Menu host' });
  });

  it('lists Duplicate and Remove, without the later-stage entries', async () => {
    await openHost(hostId);
    await openPanelMenu();
    expect(await menuItemIds('host-panel-menu')).toEqual(['host-panel-menu-duplicate', 'host-panel-menu-remove']);
    await expectAbsent(['host-panel-menu-connect', 'host-panel-menu-add-telnet']);
    await pressEscape();
    await waitForAbsent('host-panel-menu');
  });

  it('duplicates the host with " Copy" appended', async () => {
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
