// UI 13.4 Security settings: auto-lock options, change master password,
// show and reissue the recovery key (after the password), remember on this
// device (warning, hidden without a keychain), biometric unlock hidden.

import {
  answerPasswordPrompt,
  appMenu,
  closeSettings,
  el,
  expectAbsent,
  KDF_TIMEOUT,
  mock,
  newPassword,
  openSettings,
  readRecoveryKey,
  RECOVERY_KEY_PATTERN,
  restartAndUnlock,
  restartApp,
  startFresh,
  unlock,
  waitFor,
  waitForAbsent,
  waitForFieldError,
  waitForLockState,
  waitForToast,
  type Session,
} from '../helpers/index.ts';

async function changePasswordDialog(current: string, next: string, confirm: string): Promise<void> {
  await el('change-password-current').setValue(current);
  await el('change-password-new').setValue(next);
  await el('change-password-confirm').setValue(confirm);
  await el('change-password-submit').click();
}

describe('UI 13.4 Security settings', () => {
  let session: Session;

  before(async () => {
    session = await startFresh();
  });

  it('offers auto-lock after 1, 5, 15, 30 or 60 minutes or Never, 15 by default', async () => {
    await openSettings();
    const select = await waitFor('security-auto-lock');
    expect(await select.getValue()).toBe('15');
    const values = await browser.execute(
      (selector: string) =>
        Array.from((document.querySelector(selector) as HTMLSelectElement | null)?.options ?? []).map((option) => option.value),
      '[data-testid="security-auto-lock"]',
    );
    expect(values).toEqual(['1', '5', '15', '30', '60', 'never']);
  });

  it('does not show stage 6+ options (biometric unlock, sync warning)', async () => {
    await expectAbsent(['security-biometric']);
    await el('security-change-password').click();
    await waitFor('change-password-dialog');
    await expectAbsent(['change-password-sync-warning']);
    await el('change-password-cancel').click();
    await waitForAbsent('change-password-dialog');
  });

  it('validates the password change: wrong current, mismatch, too short', async () => {
    const next = newPassword();
    await el('security-change-password').click();
    await waitFor('change-password-dialog');

    await changePasswordDialog(`${session.password}-wrong`, next, next);
    await waitFor('change-password-current-error', '', KDF_TIMEOUT);

    await el('change-password-current').setValue(session.password);
    await el('change-password-new').setValue(next);
    await el('change-password-confirm').setValue(`${next}x`);
    await waitForFieldError('change-password-confirm-error', 'change-password-submit', KDF_TIMEOUT);

    await changePasswordDialog(session.password, 'short1234', 'short1234');
    await waitFor('change-password-new-error', '', KDF_TIMEOUT);
    await expect(el('change-password-dialog')).toBeExisting();
  });

  it('changes the master password, and the new one unlocks after a restart', async () => {
    const next = newPassword();
    await changePasswordDialog(session.password, next, next);
    await waitForAbsent('change-password-dialog', '', KDF_TIMEOUT * 2);
    await waitForToast('security-password-changed-toast');
    const old = session.password;
    session.password = next;
    await closeSettings();

    await restartApp();
    await el('unlock-password').setValue(old);
    await el('unlock-submit').click();
    await waitFor('unlock-password-error', '', KDF_TIMEOUT);
    await unlock(session.password);
  });

  it('shows the recovery key only after the master password, and it is the first-run key', async () => {
    await openSettings();
    await el('security-recovery-show').click();
    await waitFor('recovery-password-dialog');
    await answerPasswordPrompt(`${session.password}-wrong`);
    await waitFor('recovery-password-input-error', '', KDF_TIMEOUT);
    await answerPasswordPrompt(session.password);
    await waitFor('recovery-key-dialog', '', KDF_TIMEOUT);
    expect(await readRecoveryKey('recovery-key-text')).toBe(session.recoveryKey);
    await expectAbsent(['recovery-key-saved']);
    await expect(el('recovery-key-done')).toBeEnabled();
    await el('recovery-key-done').click();
    await waitForAbsent('recovery-key-dialog');
  });

  it('reissues the recovery key with the required checkbox; the old key stops working', async () => {
    await el('security-recovery-reissue').click();
    await waitFor('recovery-password-dialog');
    await answerPasswordPrompt(session.password);
    await waitFor('recovery-key-dialog', '', KDF_TIMEOUT);
    const reissued = await readRecoveryKey('recovery-key-text');
    expect(reissued).toMatch(RECOVERY_KEY_PATTERN);
    expect(reissued).not.toBe(session.recoveryKey);
    await expect(el('recovery-key-done')).toBeDisabled();
    await el('recovery-key-saved').click();
    await expect(el('recovery-key-done')).toBeEnabled();
    await el('recovery-key-done').click();
    await waitForAbsent('recovery-key-dialog');
    const oldKey = session.recoveryKey;
    session.recoveryKey = reissued;
    await closeSettings();

    // The old key fails in recovery unlock; the new key works.
    await appMenu('app-menu-lock');
    await waitForLockState('locked');
    await (await waitFor('unlock-use-recovery-key')).click();
    await waitFor('recovery-unlock-screen');
    const recovered = newPassword();
    await el('recovery-unlock-key').setValue(oldKey);
    await el('recovery-unlock-password').setValue(recovered);
    await el('recovery-unlock-confirm').setValue(recovered);
    await el('recovery-unlock-submit').click();
    await waitFor('recovery-unlock-key-error', '', KDF_TIMEOUT);
    await el('recovery-unlock-key').setValue(session.recoveryKey);
    await el('recovery-unlock-submit').click();
    await waitForLockState('unlocked', KDF_TIMEOUT * 2);
    session.password = recovered;
  });

  it('keeps "Remember on this device" off by default and warns before turning it on', async () => {
    await openSettings();
    const toggle = await waitFor('security-remember-device');
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await toggle.click();
    await waitFor('remember-device-warning-dialog');
    await el('remember-device-warning-cancel').click();
    await waitForAbsent('remember-device-warning-dialog');
    await expect(toggle).toHaveAttribute('aria-checked', 'false');

    await toggle.click();
    await waitFor('remember-device-warning-dialog');
    await el('remember-device-warning-confirm').click();
    await browser.waitUntil(async () => (await toggle.getAttribute('aria-checked')) === 'true', {
      timeout: KDF_TIMEOUT,
      timeoutMsg: 'remember on this device did not turn on',
    });

    // Turning it off needs no warning.
    await toggle.click();
    await browser.waitUntil(async () => (await toggle.getAttribute('aria-checked')) === 'false', {
      timeout: KDF_TIMEOUT,
      timeoutMsg: 'remember on this device did not turn off',
    });
    await expectAbsent(['remember-device-warning-dialog']);
    expect((await mock({ kind: 'secret_store_accounts' })).accounts ?? []).toEqual([]);
    await closeSettings();
  });

  it('hides "Remember on this device" when the OS has no keychain', async () => {
    await mock({ kind: 'secret_store_set_available', available: false });
    await restartAndUnlock(session.password);
    await openSettings();
    await expectAbsent(['security-remember-device', 'security-biometric']);
    await closeSettings();
    await mock({ kind: 'secret_store_set_available', available: true });
  });
});
