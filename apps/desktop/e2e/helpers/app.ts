// App-level helpers: lock states, the e2e mock command, first run, unlock,
// restart and keyboard input.

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { Key } from 'webdriverio';
import type { E2eMockAction, E2eMockResult } from '../../src/ipc/types.ts';
import { REPO_ROOT } from '../support/paths.ts';
import { el, exists, tid, waitFor } from './selectors.ts';

/** `app-root[data-lock-state]` values (stage plan, section 7.1). */
export type UiLockState = 'booting' | 'first_run' | 'locked' | 'unlocked' | 'error';

/** Time allowed for an Argon2id derivation at full strength plus UI work. */
export const KDF_TIMEOUT = 30_000;

/**
 * A master password generated for this run (architecture 9.4): 28 characters
 * from a CSPRNG, never written to disk or logs.
 */
export function newPassword(): string {
  return `E2e-${crypto.randomBytes(18).toString('base64url')}`;
}

/** Recovery key format (security.md 5): `RK1-` and 11 groups of 5 Crockford base32 characters. */
export const RECOVERY_KEY_PATTERN = /^RK1(-[0-9A-HJKMNP-TV-Z]{5}){11}$/;

/** The app version from the workspace manifest (`CARGO_PKG_VERSION` of the binary). */
export function appVersion(): string {
  const manifest = fs.readFileSync(path.join(REPO_ROOT, 'Cargo.toml'), 'utf8');
  const section = manifest.split(/^\[workspace\.package\]\s*$/m)[1] ?? '';
  const match = /^version\s*=\s*"([^"]+)"/m.exec(section);
  if (!match) {
    throw new Error('workspace version not found in Cargo.toml');
  }
  return match[1];
}

function firstLine(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.split(/\r?\n/)[0] ?? message;
}

/** True for WebDriver errors that mean the app (and so the session) is gone. */
function isSessionGone(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /invalid session id|browser has closed the connection|ECONNREFUSED/i.test(message);
}

/**
 * Polls `condition` until it returns true. Unlike `browser.waitUntil`, stops
 * at once when the session is gone (the app exited or crashed) and points to
 * the driver log, which holds the app's stderr.
 */
export async function pollUntil(
  condition: () => Promise<boolean>,
  options: { timeout: number; interval?: number; message: () => Promise<string> | string },
): Promise<void> {
  const deadline = Date.now() + options.timeout;
  let lastError: unknown;
  for (;;) {
    try {
      if (await condition()) {
        return;
      }
    } catch (error) {
      if (isSessionGone(error)) {
        throw new Error(
          `The app closed the WebDriver session (${firstLine(error)}). ` +
            `App output: ${process.env.SHELLPP_E2E_DRIVER_LOG ?? 'the tauri-driver log in target/e2e-logs'}`,
          { cause: error },
        );
      }
      lastError = error;
    }
    if (Date.now() >= deadline) {
      const reason = lastError === undefined ? '' : ` (last error: ${firstLine(lastError)})`;
      throw new Error(`${await options.message()}${reason}`);
    }
    await new Promise((resolve) => setTimeout(resolve, options.interval ?? 100));
  }
}

/** Current `data-lock-state` of the window's root, or null before it renders. */
export async function lockState(): Promise<UiLockState | null> {
  return browser.execute((selector: string) => {
    const root = document.querySelector(selector);
    return root ? (root.getAttribute('data-lock-state') as UiLockState | null) : null;
  }, tid('app-root'));
}

/** Plan name of `lockState()` (stage plan, section 8.1). */
export const appState = lockState;

/** Waits until the root reports this lock state. */
export async function waitForLockState(state: UiLockState, timeout = KDF_TIMEOUT): Promise<void> {
  await pollUntil(async () => (await lockState()) === state, {
    timeout,
    message: async () => `expected app-root[data-lock-state="${state}"], got "${String(await lockState().catch(() => null))}"`,
  });
}

/** Waits until the window has booted (any state except `booting`). */
export async function waitForBoot(timeout = 60_000): Promise<UiLockState> {
  let state: UiLockState | null = null;
  await pollUntil(
    async () => {
      state = await lockState();
      return state !== null && state !== 'booting';
    },
    {
      timeout,
      message: () =>
        `the app did not finish booting (app-root ${state === null ? 'missing' : `in state "${state}"`}); ` +
        `app output: ${process.env.SHELLPP_E2E_DRIVER_LOG ?? 'target/e2e-logs'}`,
    },
  );
  return state as unknown as UiLockState;
}

/** Calls the `e2e_mock` command through `window.__SHELLPP_E2E__` (stage plan, section 5.7). */
export async function mock(action: E2eMockAction): Promise<E2eMockResult> {
  return browser.execute(async (input: E2eMockAction) => {
    const helper = window.__SHELLPP_E2E__;
    if (!helper) {
      throw new Error('window.__SHELLPP_E2E__ is missing: is this an e2e build (vite --mode e2e)?');
    }
    return helper.mock(input);
  }, action);
}

/** Text in the mock clipboard (null or empty when cleared). */
export async function clipboardText(): Promise<string | null> {
  const result = await mock({ kind: 'clipboard_read' });
  return result.text ?? null;
}

/** URLs passed to the mock launcher, in order. */
export async function launcherRequests(): Promise<string[]> {
  return (await mock({ kind: 'launcher_requests' })).urls ?? [];
}

/** Advances the core's mock clock and runs one tick (auto-lock, clipboard clearing). */
export async function advanceClock(seconds: number): Promise<void> {
  await mock({ kind: 'clock_advance', seconds });
}

/** Window labels are `w-<n>`; the root carries the label of its window. */
export async function windowLabel(): Promise<string | null> {
  return browser.execute((selector: string) => document.querySelector(selector)?.getAttribute('data-window-label') ?? null, tid('app-root'));
}

/** `app-root[data-os]`: `windows`, `linux` or `macos`. */
export async function appOs(): Promise<string | null> {
  return browser.execute((selector: string) => document.querySelector(selector)?.getAttribute('data-os') ?? null, tid('app-root'));
}

/** Clears an input through the keyboard (select all, delete) and types new text. */
export async function clearAndType(id: string, text: string, extra = ''): Promise<void> {
  const input = el(id, extra);
  await input.waitForExist();
  await input.click();
  await selectAllAndDelete();
  if (text !== '') {
    await input.addValue(text);
  }
}

/** Select all and delete in the focused field. */
export async function selectAllAndDelete(): Promise<void> {
  await browser.keys([Key.Control, 'a']);
  await browser.keys(Key.Backspace);
}

/** Presses a key combination with Ctrl (Windows and Linux; e2e does not run on macOS). */
export async function pressCtrl(key: string): Promise<void> {
  await browser.keys([Key.Control, key]);
}

/** Presses Escape. */
export async function pressEscape(): Promise<void> {
  await browser.keys(Key.Escape);
}

/** Value of an input or select by test id. */
export async function valueOf(id: string, extra = ''): Promise<string> {
  return browser.execute((selector: string) => {
    const node = document.querySelector(selector) as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null;
    return node ? node.value : '';
  }, tid(id, extra));
}

/** True when the element with this test id has the focus. */
export async function isFocused(id: string, extra = ''): Promise<boolean> {
  return browser.execute((selector: string) => {
    const node = document.querySelector(selector);
    return node !== null && document.activeElement === node;
  }, tid(id, extra));
}

/** Waits until the element with this test id has the focus. */
export async function waitForFocus(id: string, extra = '', timeout = 5_000): Promise<void> {
  await browser.waitUntil(() => isFocused(id, extra), { timeout, timeoutMsg: `expected ${tid(id, extra)} to be focused` });
}

/** Text of a recovery key element without whitespace (the key may wrap across lines). */
export async function readRecoveryKey(id: string): Promise<string> {
  // WebDriver reports no text while the dialog's fade-in is still running.
  let key = '';
  await browser.waitUntil(
    async () => {
      key = (await el(id).getText()).replace(/\s+/g, '');
      return key !== '';
    },
    { timeout: 5_000, timeoutMsg: `${id} shows no recovery key` },
  );
  return key;
}

export interface Session {
  /** Current master password. */
  password: string;
  /** Recovery key shown by the first run (or the last reissue). */
  recoveryKey: string;
}

/**
 * Completes the first run (UI 13.1) with this password: password step,
 * recovery key step (checkbox), Finish. Returns the recovery key.
 */
export async function firstRun(password: string): Promise<string> {
  await waitForLockState('first_run', 60_000);
  await (await waitFor('first-run-password')).setValue(password);
  await el('first-run-confirm').setValue(password);
  const continueButton = el('first-run-continue');
  await continueButton.waitForEnabled();
  await continueButton.click();
  await waitFor('first-run-recovery-key', '', KDF_TIMEOUT);
  const recoveryKey = await readRecoveryKey('first-run-recovery-key');
  await el('first-run-recovery-saved').click();
  const finish = el('first-run-finish');
  await finish.waitForEnabled();
  await finish.click();
  await waitForLockState('unlocked');
  await waitForHostsReady();
  return recoveryKey;
}

/** First run with a generated password; most specs start here. */
export async function startFresh(): Promise<Session> {
  await waitForBoot();
  const password = newPassword();
  const recoveryKey = await firstRun(password);
  return { password, recoveryKey };
}

/** Waits for the Hosts screen to finish its first load. */
export async function waitForHostsReady(timeout = 20_000): Promise<void> {
  await el('hosts-screen', '[data-ready="true"]').waitForExist({ timeout });
}

/** Unlocks from the unlock screen (UI 13.2) and waits for the Hosts screen. */
export async function unlock(password: string): Promise<void> {
  await waitForLockState('locked', 60_000);
  const input = await waitFor('unlock-password');
  await input.setValue(password);
  await el('unlock-submit').click();
  await waitForLockState('unlocked');
  await waitForHostsReady();
}

/**
 * Restarts the app: ends the WebDriver session (the app process exits) and
 * starts a new one with the same data directory. Resolves after boot.
 */
export async function restartApp(): Promise<UiLockState> {
  await browser.reloadSession();
  return waitForBoot();
}

/** Restarts the app and unlocks it with the password. */
export async function restartAndUnlock(password: string): Promise<void> {
  const state = await restartApp();
  if (state !== 'unlocked') {
    await unlock(password);
  } else {
    await waitForHostsReady();
  }
}

/** True when this window shows the unlock screen. */
export async function isLockedScreen(): Promise<boolean> {
  return exists('unlock-screen');
}
