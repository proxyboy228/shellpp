// UI 1.2 Sidebar: Hosts and, from stage 2, Known Hosts; the active section
// is highlighted (`aria-current="page"`) and opens its screen; View >
// show/hide sidebar toggles the sidebar. Keychain, Port Forwarding, Snippets
// and Logs belong to later stages (hidden).

import { appMenu, el, expectAbsent, startFresh, waitFor, waitForAbsent } from '../helpers/index.ts';

const HIDDEN = [
  'sidebar-keychain', // stage 3
  'sidebar-port-forwarding', // stage 5
  'sidebar-snippets', // stage 5
  'sidebar-logs', // stage 6
];

/** Test ids of the sidebar sections in display order. */
async function sectionIds(): Promise<string[]> {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid="sidebar"] [data-testid^="sidebar-"]')).map(
      (node) => node.getAttribute('data-testid') ?? '',
    ),
  );
}

describe('UI 1.2 Sidebar', () => {
  before(async () => {
    await startFresh();
  });

  it('shows the Hosts section as the current section', async () => {
    await expect(el('sidebar')).toBeDisplayed();
    await expect(el('sidebar-hosts')).toBeDisplayed();
    await expect(el('sidebar-hosts')).toHaveAttribute('aria-current', 'page');
    await expect(el('hosts-screen')).toBeDisplayed();
  });

  it('lists Hosts and Known Hosts in this order', async () => {
    expect(await sectionIds()).toEqual(['sidebar-hosts', 'sidebar-known-hosts']);
  });

  it('opens Known Hosts and marks it as the current section', async () => {
    await el('sidebar-known-hosts').click();
    await waitFor('known-hosts-screen');
    await expect(el('sidebar-known-hosts')).toHaveAttribute('aria-current', 'page');
    await expect(el('sidebar-hosts')).not.toHaveAttribute('aria-current');
    await expectAbsent(['hosts-screen']);
    await el('sidebar-hosts').click();
    await waitFor('hosts-screen');
    await expect(el('sidebar-hosts')).toHaveAttribute('aria-current', 'page');
    await expect(el('sidebar-known-hosts')).not.toHaveAttribute('aria-current');
  });

  it('does not render the sections of later stages', async () => {
    await expectAbsent(HIDDEN);
  });

  it('hides and shows the sidebar from View > show/hide sidebar', async () => {
    await appMenu('app-menu-view', 'app-menu-view-toggle-sidebar');
    await waitForAbsent('sidebar');
    await expect(el('hosts-screen')).toBeDisplayed();
    await appMenu('app-menu-view', 'app-menu-view-toggle-sidebar');
    await waitFor('sidebar');
    await expect(el('sidebar-hosts')).toHaveAttribute('aria-current', 'page');
  });
});
