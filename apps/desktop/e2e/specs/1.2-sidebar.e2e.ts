// UI 1.2 Sidebar: the Hosts section is the only section in stage 1 and is
// highlighted; View > show/hide sidebar toggles it.

import { appMenu, el, expectAbsent, startFresh, waitFor, waitForAbsent } from '../helpers/index.ts';

const HIDDEN = [
  'sidebar-keychain', // stage 3
  'sidebar-port-forwarding', // stage 5
  'sidebar-snippets', // stage 5
  'sidebar-known-hosts', // stage 2
  'sidebar-logs', // stage 6
];

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
