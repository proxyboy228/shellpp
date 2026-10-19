// UI 2.3 Screen content: Groups above Hosts, groups as folders with
// breadcrumbs, group card count (nested), host card title and subtitle,
// selection (click, Ctrl+click, Shift+click), drag to reorder (Manual only)
// and drop onto a group card, empty state and empty folder. Stage 2: the OS
// icon is neutral until the first successful connection detects the OS, from
// the server banner or else with the detection script in an exec channel
// (UI 12.4 "OS detection").

import {
  activateVaults,
  breadcrumbIds,
  cardText,
  closePanel,
  count,
  createGroup,
  createHost,
  createSshHost,
  ctrlClick,
  dragTo,
  el,
  expectAbsent,
  goToRoot,
  groupCard,
  hostCard,
  hostIds,
  isFocused,
  openFolder,
  openSshSession,
  restartAndUnlock,
  runCommand,
  selectedHostIds,
  setSort,
  shiftClick,
  startFresh,
  waitFor,
  waitForAbsent,
  waitForGroupIds,
  waitForHostIds,
  waitForTabClosed,
  type Session,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd } from '../support/sshd.ts';

/** `data-os-family` / `data-os-distro` of a host card and of its OS icon. */
async function osOf(id: string): Promise<{ family: string | null; distro: string | null; icon: string | null; iconExists: boolean }> {
  return browser.execute((recordId: string) => {
    const card = document.querySelector(`[data-testid="host-card"][data-record-id="${recordId}"]`);
    const icon = card?.querySelector('[data-testid="host-card-os-icon"]') ?? null;
    return {
      family: card?.getAttribute('data-os-family') ?? null,
      distro: card?.getAttribute('data-os-distro') ?? null,
      icon: icon?.getAttribute('data-os-family') ?? null,
      iconExists: icon !== null,
    };
  }, id);
}

/** Connects once (accepting the host key), exits, and waits for the detected OS on the card. */
async function connectAndDetect(sshd: Sshd, id: string, distro: string): Promise<void> {
  const tab = await openSshSession(id, sshd.users[0].name, { hostKey: 'accept' });
  await runCommand('exit');
  await waitForTabClosed(tab);
  await activateVaults();
  await browser.waitUntil(async () => (await osOf(id)).distro === distro, {
    timeout: 15_000,
    timeoutMsg: `the card does not show the detected OS "${distro}"`,
  });
}

async function top(id: string): Promise<number> {
  return browser.execute(
    (selector: string) => (document.querySelector(selector) as HTMLElement).getBoundingClientRect().top,
    `[data-testid="${id}"]`,
  );
}

describe('UI 2.3 Screen content', () => {
  let session: Session;
  let infra: string;
  let net: string;
  let emptyGroup: string;
  let netHost: string;
  let infraHost: string;
  let r1: string;
  let r2: string;
  let r3: string;

  before(async () => {
    session = await startFresh();
  });

  after(async () => {
    await stopAllSshd();
  });

  it('shows the empty state with "Create host" in an empty vault', async () => {
    await waitFor('hosts-empty-state');
    await expectAbsent(['hosts-empty-import', 'hosts-groups-section', 'hosts-hosts-section']);
    await el('hosts-empty-create').click();
    await waitFor('host-panel');
    await browser.waitUntil(() => isFocused('host-panel-address'), { timeout: 5_000 });
    await closePanel('host-panel');
    await waitFor('hosts-empty-state');
  });

  it('creates the fixture tree', async () => {
    infra = await createGroup({ name: 'infra' });
    net = await createGroup({ name: 'net', parentId: infra });
    emptyGroup = await createGroup({ name: 'empty' });
    netHost = await createHost({ address: 'net.example.com', groupId: net });
    infraHost = await createHost({ address: 'infra.example.com', groupId: infra });
    r1 = await createHost({ address: 'root-1.example.com', label: 'Root host', username: 'ops', tags: ['web', 'prod'] });
    r2 = await createHost({ address: '198.51.100.7' });
    r3 = await createHost({ address: 'root-3.example.com', label: 'Third' });
    await waitForGroupIds([infra, emptyGroup], true);
    await waitForHostIds([r1, r2, r3], true);
  });

  it('shows the Groups section above the Hosts section', async () => {
    await expectAbsent(['hosts-empty-state']);
    expect(await top('hosts-groups-section')).toBeLessThan(await top('hosts-hosts-section'));
  });

  it('counts the hosts of nested groups on the group card', async () => {
    expect(await cardText('group-card', infra, 'group-card-title')).toBe('infra');
    expect(await cardText('group-card', infra, 'group-card-count')).toContain('2');
    expect(await cardText('group-card', emptyGroup, 'group-card-count')).toContain('0');
  });

  it('opens groups like folders, with breadcrumbs back to any level', async () => {
    await openFolder(infra);
    await waitForGroupIds([net]);
    await waitForHostIds([infraHost]);
    await openFolder(net);
    await waitForHostIds([netHost]);
    expect(await breadcrumbIds()).toEqual([infra, net]);
    await expect(el('hosts-breadcrumb', `[data-group-id="${net}"]`)).toHaveAttribute('aria-current');

    await el('hosts-breadcrumb', `[data-group-id="${infra}"]`).click();
    await waitForHostIds([infraHost]);
    expect(await breadcrumbIds()).toEqual([infra]);
    await goToRoot();
    await waitForHostIds([r1, r2, r3], true);
  });

  it('shows the empty-folder text in a group without content', async () => {
    await openFolder(emptyGroup);
    await waitFor('hosts-folder-empty');
    expect(await count('host-card')).toBe(0);
    await goToRoot();
  });

  it('uses the label as the card title and falls back to the address', async () => {
    expect(await cardText('host-card', r1, 'host-card-title')).toBe('Root host');
    expect(await cardText('host-card', r2, 'host-card-title')).toBe('198.51.100.7');
  });

  it('shows protocol, username, then tags in the subtitle', async () => {
    const subtitle = await cardText('host-card', r1, 'host-card-subtitle');
    const ssh = subtitle.indexOf('SSH');
    const user = subtitle.indexOf('ops');
    const tags = subtitle.indexOf('web, prod');
    expect(ssh).toBeGreaterThanOrEqual(0);
    expect(user).toBeGreaterThan(ssh);
    expect(tags).toBeGreaterThan(user);
    // Cards carry the default color (the OS icon is checked at the end).
    await expect(hostCard(r1)).toHaveAttribute('data-color', 'graphite');
    // The sync conflict badge is stage 7 (hidden), even with cards on screen.
    await expectAbsent(['host-card-conflict-badge']);
  });

  it('highlights the selected card', async () => {
    await hostCard(r1).click();
    await expect(hostCard(r1)).toHaveAttribute('data-selected', 'true');
    await waitFor('host-panel', `[data-record-id="${r1}"]`);
    expect(await selectedHostIds()).toEqual([r1]);
  });

  it('toggles cards with Ctrl+click and closes the panel for a multi-selection', async () => {
    await ctrlClick(hostCard(r3));
    await browser.waitUntil(async () => (await selectedHostIds()).length === 2);
    expect(await selectedHostIds()).toEqual([r1, r3]);
    await waitForAbsent('host-panel');
    await ctrlClick(hostCard(r1));
    await browser.waitUntil(async () => (await selectedHostIds()).join() === r3);
  });

  it('selects a range with Shift+click', async () => {
    await hostCard(r1).click();
    await browser.waitUntil(async () => (await selectedHostIds()).join() === r1);
    await shiftClick(hostCard(r3));
    await browser.waitUntil(async () => (await selectedHostIds()).length === 3, {
      timeoutMsg: 'Shift+click did not select the range',
    });
    expect(await selectedHostIds()).toEqual([r1, r2, r3]);
    await hostCard(r2).click();
    await closePanel('host-panel');
  });

  it('reorders cards by dragging in Manual mode, and the order persists', async () => {
    await expect(el('hosts-screen')).toHaveAttribute('data-sort', 'manual');
    await dragTo(hostCard(r3), hostCard(r1), 'before');
    await waitForHostIds([r3, r1, r2], true);
    await browser.pause(500);
    await restartAndUnlock(session.password);
    await waitForHostIds([r3, r1, r2], true);
  });

  it('does not reorder by dragging in Name mode', async () => {
    await setSort('name');
    const sorted = await hostIds();
    await dragTo(hostCard(sorted[2]), hostCard(sorted[0]), 'before');
    await browser.pause(1_000);
    expect(await hostIds()).toEqual(sorted);
    await setSort('manual');
    await waitForHostIds([r3, r1, r2], true);
  });

  it('moves a host into a group dropped onto the group card, in any sort mode', async () => {
    await setSort('name');
    await dragTo(hostCard(r2), groupCard(infra), 'center');
    await browser.waitUntil(async () => !(await hostIds()).includes(r2), {
      timeoutMsg: 'the host is still at the top level after the drop',
    });
    await browser.waitUntil(async () => (await cardText('group-card', infra, 'group-card-count')).includes('3'), {
      timeoutMsg: 'the group card count did not include the dropped host',
    });
    await openFolder(infra);
    await waitForHostIds([infraHost, r2]);
    await goToRoot();
    await setSort('manual');
  });

  it('shows a neutral OS icon before the first connection', async () => {
    const os = await osOf(r1);
    expect(os.iconExists).toBe(true);
    expect(os.family).toBeNull();
    expect(os.icon).toBeNull();
  });

  it('shows the OS detected from the server banner after the first connection', async () => {
    const ubuntu = await startSshd({ bannerComment: 'Ubuntu-3ubuntu13' });
    const id = await createSshHost(ubuntu, ubuntu.users[0], { label: 'Banner OS' });
    expect((await osOf(id)).family).toBeNull();
    await connectAndDetect(ubuntu, id, 'ubuntu');
    const os = await osOf(id);
    expect(os.family).toBe('linux');
    expect(os.icon).toBe('linux');
  });

  it('runs the detection script in an exec channel when the banner does not name the OS', async () => {
    const debian = await startSshd({ exec: { stdout: 'Linux\nID=debian\nVERSION_ID=12\n' } });
    const id = await createSshHost(debian, debian.users[0], { label: 'Script OS' });
    await connectAndDetect(debian, id, 'debian');
    expect((await osOf(id)).family).toBe('linux');
    const events = await debian.events();
    expect(events.some((event) => event.kind === 'exec')).toBe(true);
  });
});
