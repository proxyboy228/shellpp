// UI 12.1 Settings window (stage 1 frame): opens with Ctrl+, or the app
// menu as a modal over the window, closes with X or Esc, navigation on the
// left with only Security, external links and the version, changes apply
// immediately. The other sections and the Theme dropdown are hidden.

import {
  appMenu,
  appVersion,
  closeSettings,
  el,
  expectAbsent,
  launcherRequests,
  openSettings,
  pressCtrl,
  pressEscape,
  setAutoLock,
  startFresh,
  waitFor,
  waitForAbsent,
} from '../helpers/index.ts';

/** Test ids inside an element that start with a prefix, in document order. */
async function idsWithin(parentId: string, prefix: string): Promise<string[]> {
  return browser.execute(
    (parent: string, start: string) =>
      Array.from(document.querySelectorAll(`[data-testid="${parent}"] [data-testid^="${start}"]`)).map(
        (node) => node.getAttribute('data-testid') ?? '',
      ),
    parentId,
    prefix,
  );
}

const HIDDEN = [
  'settings-nav-account-sync', // stage 3
  'settings-nav-ssh-id', // stage 8
  'settings-nav-team', // stage 8
  'settings-nav-terminal', // stage 6
  'settings-nav-sftp', // stage 6
  'settings-nav-logs', // stage 6
  'settings-nav-shortcuts', // stage 6
  'settings-nav-updates', // stage 9
  'settings-theme-select', // stage 6
];

describe('UI 12.1 Settings window', () => {
  before(async () => {
    await startFresh();
  });

  it('opens from the app menu as a modal and closes with Esc', async () => {
    await appMenu('app-menu-settings');
    await waitFor('settings-modal');
    await expect(el('settings-modal-scrim')).toBeExisting();
    await pressEscape();
    await waitForAbsent('settings-modal');
  });

  it('opens with Ctrl+, and closes with the X button', async () => {
    await pressCtrl(',');
    await waitFor('settings-modal');
    await el('settings-modal-close').click();
    await waitForAbsent('settings-modal');
  });

  it('shows only Security in the navigation, selected, with its content', async () => {
    await openSettings();
    expect(await idsWithin('settings-nav', 'settings-nav-')).toEqual(['settings-nav-security']);
    await expect(el('settings-section-security')).toBeDisplayed();
    await expectAbsent(HIDDEN);
  });

  it('places the navigation on the left and the content on the right', async () => {
    const nav = await el('settings-nav').getLocation();
    const content = await el('settings-section-security').getLocation();
    expect(nav.x).toBeLessThan(content.x);
  });

  it('opens Documentation, Report a Bug and Changelog through the launcher', async () => {
    const before = (await launcherRequests()).length;
    for (const link of ['settings-link-documentation', 'settings-link-report-bug', 'settings-link-changelog']) {
      await el(link).click();
    }
    await browser.waitUntil(async () => (await launcherRequests()).length === before + 3, {
      timeout: 5_000,
      timeoutMsg: 'the launcher did not receive three URLs',
    });
    const urls = (await launcherRequests()).slice(before);
    for (const url of urls) {
      expect(url).toMatch(/^https:\/\/github\.com\/proxyboy228\/shellpp/);
    }
    expect(urls[1]).toContain('/issues');
    // The Settings window stays open.
    await expect(el('settings-modal')).toBeExisting();
  });

  it('shows the app version', async () => {
    expect(await el('settings-version').getText()).toContain(appVersion());
  });

  it('applies changes immediately (no Save button)', async () => {
    await setAutoLock('5');
    await closeSettings();
    await openSettings();
    expect(await el('security-auto-lock').getValue()).toBe('5');
    await setAutoLock('15');
    await closeSettings();
  });
});
