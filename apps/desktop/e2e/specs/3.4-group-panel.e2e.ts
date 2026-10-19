// UI 3.4 Group panel: Name (required), Parent Group (excludes itself and its
// subtree), Color, SSH username and port inherited by hosts and subgroups
// (nearest group wins), the group menu (Open, Edit, Move to, Remove) and
// removal of empty and non-empty groups. Vault, group credentials and host
// chaining belong to later stages (hidden).

import {
  attrOfAll,
  breadcrumbIds,
  clearAndType,
  closePanel,
  createGroup,
  createHost,
  editGroup,
  el,
  expectAbsent,
  goToRoot,
  groupCard,
  menuItemIds,
  menuPath,
  openFolder,
  openGroupMenu,
  openHost,
  openNewGroupPanel,
  pressEscape,
  saveState,
  setField,
  startFresh,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForGroupIds,
  waitForHostIds,
  waitForRecordId,
  waitForSaved,
} from '../helpers/index.ts';

const HIDDEN = [
  'group-panel-vault', // stage 8
  'group-panel-ssh-credentials', // stage 3
  'group-panel-host-chaining', // stage 5
];

const GROUP_MENU = ['group-menu-open', 'group-menu-edit', 'group-menu-move-to', 'group-menu-remove'];
const PANEL_MENU = ['group-panel-menu-open', 'group-panel-menu-edit', 'group-panel-menu-move-to', 'group-panel-menu-remove'];

describe('UI 3.4 Group panel', () => {
  let core: string;
  let edge: string;
  let other: string;
  let edgeHost: string;

  before(async () => {
    await startFresh();
  });

  it('creates a group from the Name alone', async () => {
    await openNewGroupPanel();
    expect(await saveState('group-panel')).toBe('draft');
    await expectAbsent(HIDDEN);
    await setField('group-panel-name', 'core');
    core = await waitForRecordId('group-panel');
    await waitForSaved('group-panel');
    await waitForGroupIds([core]);
  });

  it('requires the Name and keeps the last valid one', async () => {
    await clearAndType('group-panel-name', '');
    await waitFor('group-panel-name-error');
    await closePanel('group-panel');
    expect(await groupCard(core).$('[data-testid="group-card-title"]').getText()).toBe('core');
  });

  it('keeps the chosen color (graphite by default)', async () => {
    await editGroup(core);
    await expect(el('group-panel-color-graphite')).toHaveAttribute('aria-pressed', 'true');
    await el('group-panel-color-jade').click();
    await waitForSaved('group-panel');
    await closePanel('group-panel');
    await editGroup(core);
    await expect(el('group-panel-color-jade')).toHaveAttribute('aria-pressed', 'true');
  });

  it('passes SSH username and port down to hosts and subgroups, nearest group first', async () => {
    await setField('group-panel-ssh-username', 'core-user');
    await setField('group-panel-ssh-port', '2200');
    await waitForSaved('group-panel');
    await closePanel('group-panel');

    edge = await createGroup({ name: 'edge', parentId: core, port: 2300, close: false });
    const edgeUser = (await el('group-panel-ssh-username').getAttribute('placeholder')) ?? '';
    expect(edgeUser).toContain('core-user');
    expect(edgeUser).toContain('core');
    expect(await valueOf('group-panel-ssh-port')).toBe('2300');
    await closePanel('group-panel');

    edgeHost = await createHost({ address: 'edge-host.example.com', groupId: edge, close: false });
    const user = (await el('host-panel-ssh-username').getAttribute('placeholder')) ?? '';
    const port = (await el('host-panel-ssh-port').getAttribute('placeholder')) ?? '';
    expect(user).toContain('core-user');
    expect(user).toContain('core');
    expect(port).toContain('2300');
    expect(port).toContain('edge');
    await closePanel('host-panel');
  });

  it('offers parent groups without the group itself and its subtree', async () => {
    other = await createGroup({ name: 'other' });
    await editGroup(core);
    await el('group-panel-parent').click();
    await waitFor('group-panel-parent-popover');
    const options = await attrOfAll('group-panel-parent-option', 'data-group-id');
    expect(options).toContain(other);
    expect(options).not.toContain(core);
    expect(options).not.toContain(edge);
    await pressEscape();
    await waitForAbsent('group-panel-parent-popover');
    await closePanel('group-panel');
  });

  it('lists Open, Edit, Move to and Remove in the group menu and the panel menu', async () => {
    await openGroupMenu(core);
    expect(await menuItemIds('group-context-menu')).toEqual(GROUP_MENU);
    await pressEscape();
    await waitForAbsent('group-context-menu');

    await editGroup(core);
    await el('group-panel-menu-button').click();
    await waitFor('group-panel-menu');
    expect(await menuItemIds('group-panel-menu')).toEqual(PANEL_MENU);
    await pressEscape();
    await waitForAbsent('group-panel-menu');
    await closePanel('group-panel');
  });

  it('opens the group folder with Open', async () => {
    await openGroupMenu(core);
    await el('group-menu-open').click();
    await browser.waitUntil(async () => (await breadcrumbIds()).join() === core);
    await waitForGroupIds([edge]);
    await goToRoot();
  });

  it('moves a group into another group with Move to', async () => {
    await openGroupMenu(other);
    await menuPath('group-menu-move-to', `group-menu-move-to-group-${core}`);
    await waitForGroupIds([core]);
    await openFolder(core);
    await waitForGroupIds([edge, other]);
    await goToRoot();
  });

  it('removes an empty group after confirmation', async () => {
    const temp = await createGroup({ name: 'temp' });
    await waitForGroupIds([core, temp]);
    await openGroupMenu(temp);
    await el('group-menu-remove').click();
    await waitFor('group-remove-dialog');
    await expectAbsent(['group-remove-dialog-non-empty']);
    await el('group-remove-dialog-confirm').click();
    await waitForGroupIds([core]);
  });

  it('moves the content of a removed group one level up', async () => {
    await openFolder(core);
    await waitForGroupIds([edge, other]);
    await openGroupMenu(edge);
    await el('group-menu-remove').click();
    await waitFor('group-remove-dialog');
    await expect(el('group-remove-dialog-non-empty')).toBeDisplayed();
    await el('group-remove-dialog-confirm').click();
    await waitForGroupIds([other]);
    await waitForHostIds([edgeHost]);
    // The host now inherits the port from "core".
    await openHost(edgeHost);
    const port = (await el('host-panel-ssh-port').getAttribute('placeholder')) ?? '';
    expect(port).toContain('2200');
    expect(port).toContain('core');
    await closePanel('host-panel');
    await goToRoot();
  });
});
