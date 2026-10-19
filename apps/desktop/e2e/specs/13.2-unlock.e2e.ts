// UI 13.2 Unlock screen: shown at launch and after lock, covers the window
// content, focus in the password field, wrong password handling, Enter
// unlocks, "Use recovery key", and "Remember on this device".

import { Key } from 'webdriverio';
import {
  appMenu,
  appOs,
  closeSettings,
  createHost,
  el,
  exists,
  expectAbsent,
  hostCard,
  KDF_TIMEOUT,
  lockState,
  mock,
  newPassword,
  openSettings,
  restartApp,
  startFresh,
  unlock,
  valueOf,
  waitFor,
  waitForFieldError,
  waitForFocus,
  waitForHostsReady,
  waitForLockState,
  type Session,
} from '../helpers/index.ts';

describe('UI 13.2 Unlock screen', () => {
  let session: Session;
  let hostId: string;

  before(async () => {
    session = await startFresh();
    hostId = await createHost({ address: 'unlock.example.com', label: 'Unlock target' });
  });

  it('shows the unlock screen at launch, covering the window content', async () => {
    const state = await restartApp();
    expect(state).toBe('locked');
    await waitFor('unlock-screen');
    await expectAbsent(['vaults-tab', 'titlebar-tab-vaults', 'titlebar-menu-button', 'hosts-screen', 'host-card', 'settings-modal']);
    if ((await appOs()) !== 'macos') {
      expect(await exists('titlebar-window-minimize')).toBe(true);
      expect(await exists('titlebar-window-maximize')).toBe(true);
      expect(await exists('titlebar-window-close')).toBe(true);
    }
  });

  it('shows the app name, the focused password field, Unlock and "Use recovery key"', async () => {
    await expect(el('unlock-app-name')).toHaveText('Shell++');
    await waitForFocus('unlock-password');
    await expect(el('unlock-password')).toHaveAttribute('type', 'password');
    await expect(el('unlock-submit')).toBeExisting();
    await expect(el('unlock-use-recovery-key')).toBeExisting();
  });

  it('shows an error for a wrong password, clears the field and keeps the focus', async () => {
    await el('unlock-password').setValue(`${session.password}-wrong`);
    await el('unlock-submit').click();
    await waitFor('unlock-password-error', '', KDF_TIMEOUT);
    expect(await valueOf('unlock-password')).toBe('');
    await waitForFocus('unlock-password');
    expect(await lockState()).toBe('locked');
  });

  it('unlocks with Enter', async () => {
    await el('unlock-password').addValue(session.password);
    await browser.keys(Key.Enter);
    await waitForLockState('unlocked');
    await waitForHostsReady();
    await expect(hostCard(hostId)).toBeExisting();
  });

  it('goes to the recovery screen and back', async () => {
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await (await waitFor('unlock-use-recovery-key')).click();
    await waitFor('recovery-unlock-screen');
    await expect(el('recovery-unlock-key')).toBeExisting();
    await el('recovery-unlock-back').click();
    await waitFor('unlock-screen');
  });

  it('validates the recovery form', async () => {
    await (await waitFor('unlock-use-recovery-key')).click();
    await waitFor('recovery-unlock-screen');
    const fresh = newPassword();
    await el('recovery-unlock-password').setValue(fresh);
    await el('recovery-unlock-key').setValue(session.recoveryKey);
    await el('recovery-unlock-confirm').setValue(`${fresh}x`);
    await waitForFieldError('recovery-unlock-confirm-error', 'recovery-unlock-submit');
    expect(await lockState()).toBe('locked');
    await el('recovery-unlock-confirm').setValue(fresh);
    await el('recovery-unlock-key').setValue('RK1-not-a-key');
    await el('recovery-unlock-submit').click();
    await waitFor('recovery-unlock-key-error', '', KDF_TIMEOUT);
    expect(await lockState()).toBe('locked');
  });

  it('unlocks with the recovery key and a new master password', async () => {
    const newMaster = newPassword();
    await el('recovery-unlock-key').setValue(session.recoveryKey);
    await el('recovery-unlock-password').setValue(newMaster);
    await el('recovery-unlock-confirm').setValue(newMaster);
    await el('recovery-unlock-submit').click();
    await waitForLockState('unlocked', KDF_TIMEOUT * 2);
    await waitForHostsReady();
    await expect(hostCard(hostId)).toBeExisting();
    const oldPassword = session.password;
    session.password = newMaster;

    // After a restart the old password fails and the new one works.
    await restartApp();
    await waitFor('unlock-password');
    await el('unlock-password').setValue(oldPassword);
    await el('unlock-submit').click();
    await waitFor('unlock-password-error', '', KDF_TIMEOUT);
    await unlock(session.password);
    await expect(hostCard(hostId)).toBeExisting();
  });

  it('unlocks at launch without the password when "Remember on this device" is on', async () => {
    await openSettings();
    const toggle = await waitFor('security-remember-device');
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await toggle.click();
    await (await waitFor('remember-device-warning-dialog')).waitForDisplayed();
    await el('remember-device-warning-confirm').click();
    await browser.waitUntil(async () => (await toggle.getAttribute('aria-checked')) === 'true', {
      timeout: KDF_TIMEOUT,
      timeoutMsg: 'remember on this device did not turn on',
    });
    const accounts = (await mock({ kind: 'secret_store_accounts' })).accounts ?? [];
    expect(accounts.length).toBe(1);
    expect(accounts[0]).toMatch(/^device-key:/);
    await closeSettings();

    const state = await restartApp();
    expect(state).toBe('unlocked');
    await waitForHostsReady();
    await expect(hostCard(hostId)).toBeExisting();
  });

  it('still asks for the password after Lock with "Remember on this device" on', async () => {
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await waitFor('unlock-screen');
    await unlock(session.password);
  });
});
