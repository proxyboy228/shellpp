// UI 1.1 Top bar: custom frameless title bar with the app menu button, the
// persistent Vaults tab, the notification center button and the window
// buttons. Elements of later stages are not rendered.

import { appOs, el, expectAbsent, expectPresent, startFresh } from '../helpers/index.ts';

const HIDDEN = [
  'titlebar-vault-switcher', // stage 8
  'titlebar-tab-sftp', // stage 4
  'titlebar-session-tab', // stage 2
  'titlebar-new-tab', // stage 2
  'titlebar-update-button', // stage 9
  'titlebar-terminal-sidebar-toggle', // stage 2
];

async function maximized(): Promise<string | null> {
  return el('titlebar-window-maximize').getAttribute('data-maximized');
}

describe('UI 1.1 Top bar', () => {
  let os: string | null;

  before(async () => {
    await startFresh();
    os = await appOs();
  });

  it('shows the custom title bar with the Vaults tab selected', async () => {
    await expect(el('titlebar')).toBeDisplayed();
    await expect(el('titlebar-tab-vaults')).toBeDisplayed();
    await expect(el('titlebar-tab-vaults')).toHaveAttribute('aria-selected', 'true');
  });

  it('shows the notification center button without a counter when nothing is unread', async () => {
    await expect(el('titlebar-notifications-button')).toBeDisplayed();
    await expectAbsent(['titlebar-notifications-badge']);
  });

  it('shows the app menu button and the window buttons on Windows and Linux', async function () {
    if (os === 'macos') {
      // macOS uses the native menu bar and the traffic lights (manual checklist).
      await expectAbsent(['titlebar-menu-button', 'titlebar-window-minimize', 'titlebar-window-maximize', 'titlebar-window-close']);
      this.skip();
    }
    await expectPresent(['titlebar-menu-button', 'titlebar-window-minimize', 'titlebar-window-maximize', 'titlebar-window-close']);
    // Window buttons sit on the right of the bar, the menu button on the left.
    const bar = await el('titlebar').getLocation();
    const barSize = await el('titlebar').getSize();
    const menu = await el('titlebar-menu-button').getLocation();
    const close = await el('titlebar-window-close').getLocation();
    expect(menu.x - bar.x).toBeLessThan(barSize.width / 2);
    expect(close.x - bar.x).toBeGreaterThan(barSize.width / 2);
  });

  it('toggles maximize and restore with the maximize button', async function () {
    if (os === 'macos') {
      this.skip();
    }
    const before = await maximized();
    await el('titlebar-window-maximize').click();
    await browser.waitUntil(async () => (await maximized()) !== before, {
      timeout: 5_000,
      timeoutMsg: 'data-maximized did not change after maximize',
    });
    await el('titlebar-window-maximize').click();
    await browser.waitUntil(async () => (await maximized()) === before, {
      timeout: 5_000,
      timeoutMsg: 'data-maximized did not change back after restore',
    });
  });

  it('toggles maximize with a double click on the empty title bar area', async function () {
    if (os === 'macos') {
      this.skip();
    }
    const before = await maximized();
    const tab = await el('titlebar-tab-vaults').getLocation();
    const tabSize = await el('titlebar-tab-vaults').getSize();
    const close = await el('titlebar-window-minimize').getLocation();
    // A point between the Vaults tab and the right-hand buttons.
    const x = Math.round((tab.x + tabSize.width + close.x) / 2);
    const y = Math.round(tab.y + tabSize.height / 2);
    const doubleClickAt = async (): Promise<void> => {
      await browser
        .action('pointer', { parameters: { pointerType: 'mouse' } })
        .move({ origin: 'viewport', x, y })
        .down()
        .up()
        .pause(30)
        .down()
        .up()
        .perform();
    };
    await doubleClickAt();
    await browser.waitUntil(async () => (await maximized()) !== before, {
      timeout: 5_000,
      timeoutMsg: 'double click on the title bar did not toggle maximize',
    });
    await doubleClickAt();
    await browser.waitUntil(async () => (await maximized()) === before, {
      timeout: 5_000,
      timeoutMsg: 'second double click did not restore the window',
    });
  });

  it('does not render elements of later stages', async () => {
    await expectAbsent(HIDDEN);
  });
});
