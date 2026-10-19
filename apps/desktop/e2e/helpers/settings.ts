// Settings window (UI 12.1) and Security section (UI 13.4) helpers.

import { appMenu } from './menu.ts';
import { count, el, waitFor, waitForAbsent } from './selectors.ts';

/** Opens the Settings window from the app menu. */
export async function openSettings(): Promise<void> {
  if ((await count('settings-modal')) > 0) {
    return;
  }
  await appMenu('app-menu-settings');
  await waitFor('settings-modal');
  await waitFor('settings-section-security');
}

/** Closes the Settings window with its X button. */
export async function closeSettings(): Promise<void> {
  if ((await count('settings-modal')) === 0) {
    return;
  }
  await el('settings-modal-close').click();
  await waitForAbsent('settings-modal');
}

/** Auto-lock option values (UI 13.4). */
export type AutoLockValue = '1' | '5' | '15' | '30' | '60' | 'never';

/** Picks an auto-lock interval in the open Settings window. */
export async function setAutoLock(value: AutoLockValue): Promise<void> {
  const select = await waitFor('security-auto-lock');
  await select.selectByAttribute('value', value);
  await browser.waitUntil(async () => (await select.getValue()) === value, {
    timeoutMsg: `auto-lock did not change to ${value}`,
  });
}

/** Opens Settings, picks the auto-lock interval and closes Settings. */
export async function configureAutoLock(value: AutoLockValue): Promise<void> {
  await openSettings();
  await setAutoLock(value);
  await closeSettings();
}

/**
 * Answers the master password prompt of Show / Reissue recovery key
 * (`recovery-password-dialog`).
 */
export async function answerPasswordPrompt(password: string): Promise<void> {
  const input = await waitFor('recovery-password-input');
  await input.setValue(password);
  await el('recovery-password-submit').click();
}
