// UI 13.3 Lock and auto-lock: Lock from the app menu locks every window,
// every window returns to its state after unlock, auto-lock after the
// interval without input (mock clock), input resets the timer, Never.

import {
  advanceClock,
  appMenu,
  breadcrumbIds,
  closeSettings,
  configureAutoLock,
  expectAbsent,
  createGroup,
  createHost,
  el,
  goToRoot,
  hostCard,
  lockState,
  openFolder,
  openHost,
  openSettings,
  search,
  selectedHostIds,
  setAutoLock,
  startFresh,
  unlock,
  waitFor,
  waitForBoot,
  waitForLockState,
  type Session,
} from '../helpers/index.ts';

/** Activity is reported at most every 30 s (stage plan, src/app/activity.ts). */
const ACTIVITY_THROTTLE_MS = 30_000;

async function switchToLabel(label: string): Promise<void> {
  await browser.waitUntil(
    async () => {
      for (const handle of await browser.getWindowHandles()) {
        await browser.switchToWindow(handle);
        const current = await browser.execute(
          () => document.querySelector('[data-testid="app-root"]')?.getAttribute('data-window-label') ?? null,
        );
        if (current === label) {
          return true;
        }
      }
      return false;
    },
    { timeout: 20_000, timeoutMsg: `window ${label} not found` },
  );
}

describe('UI 13.3 Lock and auto-lock', () => {
  let session: Session;
  let rootHost: string;
  let groupId: string;
  let nestedHost: string;

  before(async () => {
    session = await startFresh();
    rootHost = await createHost({ address: 'lock-root.example.com', label: 'Lock root' });
    groupId = await createGroup({ name: 'lock-group' });
    nestedHost = await createHost({ address: 'lock-nested.example.com', label: 'Lock nested', groupId });
  });

  it('locks every window at once from the app menu', async () => {
    await appMenu('app-menu-window', 'app-menu-window-new-window');
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 2, {
      timeout: 20_000,
      timeoutMsg: 'the second window did not open',
    });
    await switchToLabel('w-2');
    await waitForLockState('unlocked');

    await switchToLabel('w-1');
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await waitFor('unlock-screen');

    await switchToLabel('w-2');
    await waitForLockState('locked');
    await waitFor('unlock-screen');

    // Unlocking in one window unlocks every window.
    await unlock(session.password);
    await switchToLabel('w-1');
    await waitForLockState('unlocked');

    await switchToLabel('w-2');
    await el('titlebar-window-close').click();
    await browser.waitUntil(async () => (await browser.getWindowHandles()).length === 1, {
      timeout: 10_000,
      timeoutMsg: 'the second window did not close',
    });
    await browser.switchToWindow((await browser.getWindowHandles())[0]);
    await waitForBoot();
  });

  it('restores the folder, the selection and the open panel after unlock', async () => {
    await openFolder(groupId);
    await openHost(nestedHost);
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await unlock(session.password);
    expect(await breadcrumbIds()).toEqual([groupId]);
    expect(await selectedHostIds()).toEqual([nestedHost]);
    await expect(el('host-panel', `[data-record-id="${nestedHost}"]`)).toBeExisting();
    await el('host-panel-collapse').click();
    await goToRoot();
  });

  it('locks automatically after the auto-lock interval without input', async () => {
    await configureAutoLock('1');
    await hostCard(rootHost).click();
    await advanceClock(61);
    await waitForLockState('locked');
    await waitFor('unlock-screen');
    await unlock(session.password);
  });

  it('reopens the Settings window after unlock when it was open', async () => {
    await openSettings();
    await setAutoLock('1');
    await advanceClock(61);
    await waitForLockState('locked');
    await expectAbsent(['settings-modal']);
    await unlock(session.password);
    await waitFor('settings-modal');
    await closeSettings();
  });

  it('counts keyboard input in the app window as activity', async () => {
    // The mock clock is system time plus an offset, so real waiting moves it
    // too: use a 5 minute interval that the 31 s pause below cannot reach.
    await configureAutoLock('5');
    // Let the activity throttle window pass, then type: this input resets the timer.
    await browser.pause(ACTIVITY_THROTTLE_MS + 1_000);
    await search('a');
    // About 280 s after the input (and more than 300 s after the unlock).
    await advanceClock(280);
    await browser.pause(1_500);
    expect(await lockState()).toBe('unlocked');
    await advanceClock(25);
    await waitForLockState('locked');
    await unlock(session.password);
  });

  it('never locks automatically with "Never"', async () => {
    await configureAutoLock('never');
    await advanceClock(86_400);
    await browser.pause(1_500);
    expect(await lockState()).toBe('unlocked');
  });
});
