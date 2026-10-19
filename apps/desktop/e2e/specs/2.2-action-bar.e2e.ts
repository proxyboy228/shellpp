// UI 2.2 Action bar: New host split button with New Group (in the current
// folder), view toggle tiles/list (persisted), tag filter (multi-select, AND,
// chips) and the sort menu (persisted). Import, cloud integrations, Terminal,
// Serial and vault members belong to later stages.

import {
  breadcrumbIds,
  closePanel,
  count,
  createGroup,
  createHost,
  el,
  expectAbsent,
  goToRoot,
  groupIds,
  isFocused,
  openFolder,
  openHost,
  openNewGroupPanel,
  openNewHostPanel,
  pressEscape,
  restartAndUnlock,
  setField,
  setSort,
  startFresh,
  waitFor,
  waitForAbsent,
  waitForGroupIds,
  waitForHostIds,
  waitForRecordId,
  waitForSaved,
  type Session,
} from '../helpers/index.ts';

async function openTagFilter(): Promise<void> {
  if ((await count('hosts-tag-filter-menu')) === 0) {
    await el('hosts-tag-filter-button').click();
  }
  await waitFor('hosts-tag-filter-menu');
}

async function closeTagFilter(): Promise<void> {
  if ((await count('hosts-tag-filter-menu')) > 0) {
    await pressEscape();
    await waitForAbsent('hosts-tag-filter-menu');
  }
}

async function toggleTag(tag: string): Promise<void> {
  await openTagFilter();
  await el('hosts-tag-filter-option', `[data-tag="${tag}"]`).click();
}

describe('UI 2.2 Action bar', () => {
  let session: Session;

  before(async () => {
    session = await startFresh();
  });

  it('opens the host panel with the focus in Address from New host', async () => {
    await openNewHostPanel();
    await browser.waitUntil(() => isFocused('host-panel-address'), {
      timeout: 5_000,
      timeoutMsg: 'Address is not focused in a new host panel',
    });
    await closePanel('host-panel');
  });

  it('does not render later-stage actions', async () => {
    await expectAbsent(['hosts-local-terminal-button', 'hosts-serial-button', 'hosts-vault-members']);
    await el('hosts-new-host-toggle').click();
    await waitFor('hosts-new-host-menu');
    await expect(el('hosts-new-host-item-new-group')).toBeExisting();
    await expectAbsent(['hosts-new-host-item-import', 'hosts-new-host-item-cloud']);
    await pressEscape();
    await waitForAbsent('hosts-new-host-menu');
  });

  it('creates a group in the current folder with New Group', async () => {
    await openNewGroupPanel();
    await expect(el('group-panel')).not.toHaveAttribute('data-record-id');
    await setField('group-panel-name', 'alpha');
    const alpha = await waitForRecordId('group-panel');
    await waitForSaved('group-panel');
    await closePanel('group-panel');
    await waitForGroupIds([alpha]);

    await openFolder(alpha);
    await openNewGroupPanel();
    await setField('group-panel-name', 'beta');
    const beta = await waitForRecordId('group-panel');
    await waitForSaved('group-panel');
    await closePanel('group-panel');
    await waitForGroupIds([beta]);
    expect(await breadcrumbIds()).toEqual([alpha]);

    await goToRoot();
    await waitForGroupIds([alpha]);
    expect(await groupIds()).not.toContain(beta);
  });

  it('switches between tiles and list, and keeps the choice after a restart', async () => {
    await createHost({ address: 'layout.example.com' });
    await expect(el('hosts-screen')).toHaveAttribute('data-layout', 'grid');
    await el('hosts-view-toggle-list').click();
    await expect(el('hosts-screen')).toHaveAttribute('data-layout', 'list');
    await browser.pause(500);
    await restartAndUnlock(session.password);
    await expect(el('hosts-screen')).toHaveAttribute('data-layout', 'list');
    await el('hosts-view-toggle-grid').click();
    await expect(el('hosts-screen')).toHaveAttribute('data-layout', 'grid');
  });

  it('filters by tags with AND and shows active filters as removable chips', async () => {
    const both = await createHost({ address: 'tags-both.example.com', tags: ['tf-red', 'tf-blue'] });
    const red = await createHost({ address: 'tags-red.example.com', tags: ['tf-red'] });

    await toggleTag('tf-red');
    await waitForHostIds([both, red]);
    await openTagFilter();
    await expect(el('hosts-tag-filter-option', '[data-tag="tf-red"]')).toHaveAttribute('aria-checked', 'true');
    await expect(el('hosts-tag-filter-option', '[data-tag="tf-blue"]')).toHaveAttribute('aria-checked', 'false');
    await toggleTag('tf-blue');
    await waitForHostIds([both]);
    await closeTagFilter();
    expect(await count('hosts-tag-filter-chip')).toBe(2);
    await expect(el('hosts-tag-filter-chip', '[data-tag="tf-red"]')).toBeDisplayed();

    // Removing the red chip leaves the blue filter.
    await el('hosts-tag-filter-chip', '[data-tag="tf-red"]').$('[data-testid="hosts-tag-filter-chip-remove"]').click();
    await waitForAbsent('hosts-tag-filter-chip', '[data-tag="tf-red"]');
    await waitForHostIds([both]);
    await el('hosts-tag-filter-chip', '[data-tag="tf-blue"]').$('[data-testid="hosts-tag-filter-chip-remove"]').click();
    await waitForAbsent('hosts-tag-filter-chip');
    // Back to the folder view: the root shows every top-level host again
    // (once the unfiltered query has answered).
    await browser.waitUntil(async () => (await count('host-card')) >= 3, {
      timeout: 5_000,
      timeoutMsg: 'the root did not show every top-level host after the last tag chip was removed',
    });
  });

  it('sorts by Manual, Name, Created and Modified, and keeps the mode after a restart', async () => {
    const folder = await createGroup({ name: 'sorting' });
    const charlie = await createHost({ address: 'sort-c.example.com', label: 'charlie', groupId: folder });
    const alpha = await createHost({ address: 'sort-a.example.com', label: 'alpha', groupId: folder });
    const bravo = await createHost({ address: 'sort-b.example.com', label: 'bravo', groupId: folder });
    await openFolder(folder);

    await expect(el('hosts-screen')).toHaveAttribute('data-sort', 'manual');
    await waitForHostIds([charlie, alpha, bravo], true);
    await setSort('name');
    await waitForHostIds([alpha, bravo, charlie], true);
    await expect(el('hosts-sort-button')).toHaveAttribute('data-sort', 'name');
    await setSort('created');
    await waitForHostIds([bravo, alpha, charlie], true);

    await openHost(charlie);
    await setField('host-panel-label', 'charlie 2');
    await waitForSaved('host-panel');
    await closePanel('host-panel');
    await setSort('modified');
    await waitForHostIds([charlie, bravo, alpha], true);

    await browser.pause(500);
    await restartAndUnlock(session.password);
    await expect(el('hosts-screen')).toHaveAttribute('data-sort', 'modified');
    await setSort('manual');
    await openFolder(folder);
    await waitForHostIds([charlie, alpha, bravo], true);
  });
});
