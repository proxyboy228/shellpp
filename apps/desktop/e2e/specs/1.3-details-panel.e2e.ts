// UI 1.3 Details panel: slides out on the right and narrows the main area,
// header with title, "..." menu and collapse, resizable 320-560 px with the
// width persisted, and the saving rule (draft, autosave, invalid values not
// saved, closing discards a draft).

import {
  clearAndType,
  closePanel,
  count,
  createHost,
  dragBy,
  el,
  hostIds,
  openHost,
  openNewHostPanel,
  restartAndUnlock,
  startFresh,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForSaved,
  type Session,
} from '../helpers/index.ts';

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

async function box(id: string): Promise<Box> {
  return browser.execute((selector: string) => {
    const rect = (document.querySelector(selector) as HTMLElement).getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  }, `[data-testid="${id}"]`);
}

describe('UI 1.3 Details panel', () => {
  let session: Session;
  let hostId: string;

  before(async () => {
    session = await startFresh();
    hostId = await createHost({ address: 'panel-host.example.com', label: 'Panel host' });
  });

  it('opens on the right when a host is selected and narrows the main area without overlap', async () => {
    const searchBefore = await box('hosts-search-input');
    await openHost(hostId);
    // The panel is at the right edge of the screen area once its slide-in
    // animation (a short translateX) has finished.
    const rightEdgeGap = async (): Promise<number> => {
      const panelBox = await box('host-panel');
      const screenBox = await box('hosts-screen');
      return Math.abs(panelBox.x + panelBox.width - (screenBox.x + screenBox.width));
    };
    await browser.waitUntil(async () => (await rightEdgeGap()) <= 2, {
      timeout: 5_000,
      timeoutMsg: 'the details panel did not settle at the right edge of the screen area',
    });
    const panel = await box('host-panel');
    await browser.waitUntil(async () => (await box('hosts-search-input')).width < searchBefore.width - 100, {
      timeout: 5_000,
      timeoutMsg: 'the main area did not narrow when the panel opened',
    });
    const searchAfter = await box('hosts-search-input');
    expect(searchAfter.x + searchAfter.width).toBeLessThanOrEqual(panel.x + 1);
  });

  it('shows the title, the "..." menu button and the collapse button in the header', async () => {
    await expect(el('host-panel-title')).toBeDisplayed();
    expect((await el('host-panel-title').getText()).trim()).not.toBe('');
    await expect(el('host-panel-menu-button')).toBeDisplayed();
    await expect(el('host-panel-collapse')).toBeDisplayed();
  });

  it('closes with the collapse button and the main area takes the width back', async () => {
    const narrow = await box('hosts-search-input');
    await el('host-panel-collapse').click();
    await waitForAbsent('host-panel');
    await browser.waitUntil(async () => (await box('hosts-search-input')).width > narrow.width + 100, {
      timeout: 5_000,
      timeoutMsg: 'the main area did not widen after closing the panel',
    });
  });

  it('resizes with the handle within 320-560 px and keeps the width after a restart', async () => {
    await openHost(hostId);
    const start = (await box('host-panel')).width;
    expect(start).toBeGreaterThanOrEqual(320);
    expect(start).toBeLessThanOrEqual(560);
    // The handle is on the left edge: dragging left widens the panel.
    const delta = start + 60 <= 560 ? 60 : -60;
    await dragBy(el('host-panel-resize-handle'), -delta);
    await browser.waitUntil(async () => Math.abs((await box('host-panel')).width - (start + delta)) <= 4, {
      timeout: 5_000,
      timeoutMsg: `panel width did not change by ${delta} px`,
    });
    const resized = (await box('host-panel')).width;

    // Dragging far beyond the limits clamps to 320-560.
    await dragBy(el('host-panel-resize-handle'), -400);
    await browser.waitUntil(async () => Math.abs((await box('host-panel')).width - 560) <= 2, {
      timeout: 5_000,
      timeoutMsg: 'panel width is not clamped to 560 px',
    });
    await dragBy(el('host-panel-resize-handle'), 400);
    await browser.waitUntil(async () => Math.abs((await box('host-panel')).width - 320) <= 2, {
      timeout: 5_000,
      timeoutMsg: 'panel width is not clamped to 320 px',
    });
    await dragBy(el('host-panel-resize-handle'), -(resized - 320));
    await browser.waitUntil(async () => Math.abs((await box('host-panel')).width - resized) <= 4, {
      timeout: 5_000,
      timeoutMsg: 'panel width did not return to the resized width',
    });
    const persisted = (await box('host-panel')).width;
    // The width is a setting (view.details_panel_width): give the write a moment.
    await browser.pause(1_000);

    await restartAndUnlock(session.password);
    await openHost(hostId);
    expect(Math.abs((await box('host-panel')).width - persisted)).toBeLessThanOrEqual(2);
    await closePanel('host-panel');
  });

  it('discards a new host draft without an address when the panel is closed', async () => {
    const before = await hostIds();
    await openNewHostPanel();
    await expect(el('host-panel')).not.toHaveAttribute('data-record-id');
    await el('host-panel-label').setValue('Draft without address');
    await browser.pause(800); // longer than the autosave debounce
    await expect(el('host-panel')).toHaveAttribute('data-save-state', 'draft');
    await closePanel('host-panel');
    expect(await hostIds()).toEqual(before);
    expect(await count('host-card')).toBe(before.length);
  });

  it('shows an invalid value with the reason and keeps the last valid value', async () => {
    await openHost(hostId);
    await clearAndType('host-panel-ssh-port', '2222');
    await waitForSaved('host-panel');
    await clearAndType('host-panel-ssh-port', '70000');
    await waitFor('host-panel-ssh-port-error');
    expect((await el('host-panel-ssh-port-error').getText()).trim()).not.toBe('');
    await closePanel('host-panel');
    await openHost(hostId);
    expect(await valueOf('host-panel-ssh-port')).toBe('2222');
    await expect(el('host-panel-ssh-port-error')).not.toBeExisting();
    await closePanel('host-panel');
  });
});
