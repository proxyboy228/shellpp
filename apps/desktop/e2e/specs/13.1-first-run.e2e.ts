// UI 13.1 First run: master password with confirmation, permanent warning,
// recovery key step with Copy (cleared after 30 s) and the required checkbox,
// Finish opens the empty Hosts screen.

import {
  advanceClock,
  appOs,
  clipboardText,
  el,
  exists,
  expectAbsent,
  KDF_TIMEOUT,
  lockState,
  newPassword,
  readRecoveryKey,
  RECOVERY_KEY_PATTERN,
  restartApp,
  waitFor,
  waitForBoot,
  waitForHostsReady,
  waitForLockState,
} from '../helpers/index.ts';

describe('UI 13.1 First run', () => {
  const password = newPassword();
  let recoveryKey = '';

  before(async () => {
    await waitForBoot();
  });

  it('shows the first-run screen before any other screen on the first launch', async () => {
    expect(await lockState()).toBe('first_run');
    await waitFor('first-run-screen');
    await expect(el('first-run-warning')).toBeDisplayed();
    // Only the minimal title bar: no tabs, no app menu, no main content.
    await expectAbsent(['vaults-tab', 'titlebar-tab-vaults', 'titlebar-menu-button', 'hosts-screen', 'unlock-screen']);
    if ((await appOs()) !== 'macos') {
      expect(await exists('titlebar-window-close')).toBe(true);
    }
  });

  it('keeps Continue disabled until both fields are filled in and match', async () => {
    const continueButton = el('first-run-continue');
    await expect(continueButton).toBeDisabled();
    await el('first-run-password').setValue(password);
    await expect(continueButton).toBeDisabled();
    await el('first-run-confirm').setValue(`${password}x`);
    await expect(continueButton).toBeDisabled();
    await waitFor('first-run-confirm-error');
    await el('first-run-confirm').setValue(password);
    await expect(continueButton).toBeEnabled();
    await expect(el('first-run-confirm-error')).not.toBeExisting();
  });

  it('masks both fields and shows them with the show/hide buttons', async () => {
    for (const field of ['first-run-password', 'first-run-confirm']) {
      await expect(el(field)).toHaveAttribute('type', 'password');
      await el(`${field}-toggle`).click();
      await expect(el(field)).toHaveAttribute('type', 'text');
      await el(`${field}-toggle`).click();
      await expect(el(field)).toHaveAttribute('type', 'password');
    }
  });

  it('shows the core error under the field for a password shorter than 10 characters', async () => {
    const short = 'Ab1-xyz9'; // 8 characters
    await el('first-run-password').setValue(short);
    await el('first-run-confirm').setValue(short);
    await el('first-run-continue').waitForEnabled();
    await el('first-run-continue').click();
    await waitFor('first-run-password-error', '', KDF_TIMEOUT);
    expect(await exists('first-run-recovery-step')).toBe(false);
    expect(await lockState()).toBe('first_run');
  });

  it('keeps the unrecoverable-data warning visible (not dismissible)', async () => {
    await expect(el('first-run-warning')).toBeDisplayed();
    const closeControls = await browser.execute(
      (selector: string) => document.querySelector(selector)?.querySelectorAll('button').length ?? -1,
      '[data-testid="first-run-warning"]',
    );
    expect(closeControls).toBe(0);
  });

  it('continues to the recovery key step with a valid password', async () => {
    await el('first-run-password').setValue(password);
    await el('first-run-confirm').setValue(password);
    await el('first-run-continue').waitForEnabled();
    await el('first-run-continue').click();
    await waitFor('first-run-recovery-step', '', KDF_TIMEOUT);
    recoveryKey = await readRecoveryKey('first-run-recovery-key');
    expect(recoveryKey).toMatch(RECOVERY_KEY_PATTERN);
    const fontFamily = (await el('first-run-recovery-key').getCSSProperty('font-family')).value ?? '';
    expect(fontFamily.toLowerCase()).toMatch(/mono/);
  });

  it('copies the recovery key and clears the clipboard after 30 seconds', async () => {
    await el('first-run-recovery-copy').click();
    await waitFor('first-run-copied-toast');
    expect(await clipboardText()).toBe(recoveryKey);
    await advanceClock(20);
    expect(await clipboardText()).toBe(recoveryKey);
    await advanceClock(11);
    await browser.waitUntil(async () => !(await clipboardText()), {
      timeout: 5_000,
      timeoutMsg: 'the clipboard still holds the recovery key after 31 s',
    });
  });

  it('keeps Finish disabled until "I saved the recovery key" is checked', async () => {
    const finish = el('first-run-finish');
    await expect(finish).toBeDisabled();
    await el('first-run-recovery-saved').click();
    await expect(el('first-run-recovery-saved')).toBeSelected();
    await expect(finish).toBeEnabled();
    await el('first-run-recovery-saved').click();
    await expect(finish).toBeDisabled();
    await el('first-run-recovery-saved').click();
    await expect(finish).toBeEnabled();
  });

  it('opens the Vaults tab with the empty Hosts screen after Finish', async () => {
    await el('first-run-finish').click();
    await waitForLockState('unlocked');
    await waitForHostsReady();
    await expect(el('vaults-tab')).toBeExisting();
    await expect(el('titlebar-tab-vaults')).toHaveAttribute('aria-selected', 'true');
    await expect(el('hosts-empty-state')).toBeExisting();
    await expectAbsent(['first-run-screen', 'first-run-recovery-step', 'host-card', 'group-card']);
  });

  it('asks for the master password (not the first run) at the next launch', async () => {
    const state = await restartApp();
    expect(state).toBe('locked');
    await waitFor('unlock-screen');
    await expectAbsent(['first-run-screen']);
  });
});
