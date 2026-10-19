// UI 3.2 Host panel behavior: the address is the only required field, the
// host appears as soon as the address is valid, changes save automatically
// (no save button), an empty address with close discards the draft, an
// invalid value is not saved.

import {
  cardText,
  clearAndType,
  closePanel,
  count,
  createHost,
  el,
  hostCard,
  hostIds,
  openHost,
  openNewHostPanel,
  restartAndUnlock,
  saveState,
  setField,
  startFresh,
  valueOf,
  waitFor,
  waitForRecordId,
  waitForSaved,
  type Session,
} from '../helpers/index.ts';

/** Records every `data-save-state` value of the host panel from now on (in the page). */
async function recordSaveStates(): Promise<void> {
  await browser.execute(() => {
    const target = document.querySelector('[data-testid="host-panel"]');
    const store = window as unknown as { __e2eSaveStates?: string[]; __e2eSaveObserver?: MutationObserver };
    store.__e2eSaveObserver?.disconnect();
    store.__e2eSaveStates = [target?.getAttribute('data-save-state') ?? ''];
    if (!target) {
      return;
    }
    store.__e2eSaveObserver = new MutationObserver(() => {
      store.__e2eSaveStates?.push(target.getAttribute('data-save-state') ?? '');
    });
    store.__e2eSaveObserver.observe(target, { attributes: true, attributeFilter: ['data-save-state'] });
  });
}

async function recordedSaveStates(): Promise<string[]> {
  return browser.execute(() => {
    const store = window as unknown as { __e2eSaveStates?: string[]; __e2eSaveObserver?: MutationObserver };
    store.__e2eSaveObserver?.disconnect();
    return store.__e2eSaveStates ?? [];
  });
}

describe('UI 3.2 Host panel behavior', () => {
  let session: Session;
  let hostId: string;

  before(async () => {
    session = await startFresh();
  });

  it('adds the host to the list as soon as a valid address is typed', async () => {
    await openNewHostPanel();
    expect(await saveState('host-panel')).toBe('draft');
    await expect(el('host-panel')).not.toHaveAttribute('data-record-id');
    expect(await count('host-card')).toBe(0);
    await el('host-panel-address').click();
    await el('host-panel-address').addValue('auto.example.com');
    hostId = await waitForRecordId('host-panel');
    // No save button: the card shows up while the panel is still open.
    await waitFor('host-card', `[data-record-id="${hostId}"]`);
    await waitForSaved('host-panel');
  });

  it('moves through the save states pending, saving and saved', async () => {
    await recordSaveStates();
    await setField('host-panel-label', 'Autosaved');
    await waitForSaved('host-panel');
    const states = await recordedSaveStates();
    expect(states.at(-1)).toBe('saved');
    expect(states).toContain('pending');
    expect(states.indexOf('pending')).toBeLessThan(states.lastIndexOf('saved'));
    // The list re-queries after the save event, so the card follows shortly.
    await browser.waitUntil(async () => (await cardText('host-card', hostId, 'host-card-title')) === 'Autosaved', {
      timeout: 5_000,
      timeoutMsg: 'the host card title did not change to the saved label',
    });
  });

  it('keeps the edits after a restart', async () => {
    await setField('host-panel-label', 'Persisted label');
    await setField('host-panel-ssh-username', 'persist');
    await waitForSaved('host-panel');
    await closePanel('host-panel');
    await restartAndUnlock(session.password);
    expect(await cardText('host-card', hostId, 'host-card-title')).toBe('Persisted label');
    await openHost(hostId);
    expect(await valueOf('host-panel-ssh-username')).toBe('persist');
    expect(await valueOf('host-panel-address')).toBe('auto.example.com');
    await closePanel('host-panel');
  });

  it('shows an invalid address error and keeps the last valid address', async () => {
    const second = await createHost({ address: 'second.example.com', close: false });
    await clearAndType('host-panel-address', 'not valid');
    await waitFor('host-panel-address-error');
    await browser.pause(800); // longer than the autosave debounce
    await closePanel('host-panel');
    expect(await cardText('host-card', second, 'host-card-title')).toBe('second.example.com');
    await openHost(second);
    expect(await valueOf('host-panel-address')).toBe('second.example.com');
  });

  it('treats an emptied address as invalid (required) and keeps the host', async () => {
    const second = (await hostIds()).find((id) => id !== hostId) as string;
    await clearAndType('host-panel-address', '');
    await waitFor('host-panel-address-error');
    await closePanel('host-panel');
    await expect(hostCard(second)).toBeExisting();
    expect(await cardText('host-card', second, 'host-card-title')).toBe('second.example.com');
  });

  it('discards a draft with an empty address when the panel is closed', async () => {
    const before = await hostIds();
    await openNewHostPanel();
    await el('host-panel-label').setValue('Only a label');
    await el('host-panel-ssh-username').setValue('nobody');
    await browser.pause(800);
    expect(await saveState('host-panel')).toBe('draft');
    await closePanel('host-panel');
    expect(await hostIds()).toEqual(before);
  });
});
