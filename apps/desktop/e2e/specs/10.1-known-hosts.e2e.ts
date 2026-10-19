// UI 10.1 Known Hosts: the sidebar section opens the screen; an accepted
// host key becomes a card with the OpenSSH address (`[host]:port`) and the
// key type; the details panel shows address, port, key type, the SHA256
// fingerprint (copy), the date added, the saved hosts with this address and
// Remove; search, view toggle and sort persist; the context menu copies the
// fingerprint and removes; a hashed entry shows "Hashed entry" and matches on
// connect without a question; a revoked entry has the revoked badge. Import
// and Export belong to stage 3 (hidden).

import {
  activateVaults,
  clearAndType,
  clipboardText,
  connectHost,
  createSshHost,
  el,
  expectAbsent,
  existsInPane,
  hashedHost,
  knownHostCard,
  knownHostCardText,
  knownHostIds,
  openHostsScreen,
  openKnownHosts,
  openSshSession,
  paneState,
  pollUntil,
  restartAndUnlock,
  rightClick,
  runCommand,
  seedKnownHost,
  startFresh,
  textAt,
  tid,
  waitFor,
  waitForAbsent,
  waitForKnownHostCount,
  waitForShell,
  waitForTabClosed,
  waitForToast,
  type Session,
} from '../helpers/index.ts';
import { keygen, startSshd, stopAllSshd, type GeneratedKey, type Sshd, type SshdUser } from '../support/sshd.ts';

const HIDDEN = [
  'known-hosts-import', // stage 3
  'known-hosts-export', // stage 3
];

function panelPart(id: string): string {
  return `${tid('known-host-panel')} ${tid(id)}`;
}

describe('UI 10.1 Known Hosts', () => {
  let session: Session;
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;
  let tofuId: string;
  let seeded: GeneratedKey;
  let seededId: string;

  before(async () => {
    session = await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'Known host' });
  });

  after(async () => {
    await stopAllSshd();
  });

  it('opens the empty screen from the sidebar', async () => {
    await openKnownHosts();
    await expect(el('sidebar-known-hosts')).toHaveAttribute('aria-current', 'page');
    await expect(el('known-hosts-screen')).toBeDisplayed();
    await waitFor('known-hosts-empty');
    await expectAbsent(HIDDEN);
  });

  it('shows an accepted host key as a card with the OpenSSH address and the key type', async () => {
    await openHostsScreen();
    const tab = await openSshSession(hostId, user.name, { hostKey: 'accept' });
    await runCommand('exit');
    await waitForTabClosed(tab);
    await openKnownHosts();
    [tofuId] = await waitForKnownHostCount(1);
    expect(await knownHostCardText(tofuId, 'known-host-card-title')).toBe(`[127.0.0.1]:${sshd.port}`);
    expect(await knownHostCardText(tofuId, 'known-host-card-subtitle')).toContain(sshd.hostKeyType);
    await expect(knownHostCard(tofuId)).toHaveAttribute('data-hashed', 'false');
    await expect(knownHostCard(tofuId)).toHaveAttribute('data-revoked', 'false');
  });

  it('shows the details of an entry and copies its fingerprint', async () => {
    await knownHostCard(tofuId).click();
    await waitFor('known-host-panel', `[data-record-id="${tofuId}"]`);
    await expect(knownHostCard(tofuId)).toHaveAttribute('data-selected', 'true');
    expect(await textAt(panelPart('known-host-panel-address'))).toContain('127.0.0.1');
    expect(await textAt(panelPart('known-host-panel-port'))).toContain(String(sshd.port));
    expect(await textAt(panelPart('known-host-panel-key-type'))).toContain(sshd.hostKeyType);
    expect(await textAt(panelPart('known-host-panel-fingerprint'))).toBe(sshd.hostKeyFingerprint);
    expect((await textAt(panelPart('known-host-panel-added'))).length).toBeGreaterThan(0);

    await el('known-host-panel-fingerprint-copy').click();
    await waitForToast('known-hosts-fingerprint-copied-toast');
    await browser.waitUntil(async () => (await clipboardText()) === sshd.hostKeyFingerprint, {
      timeout: 5_000,
      timeoutMsg: 'the fingerprint was not copied',
    });
  });

  it('lists the saved hosts with this address and opens one', async () => {
    const host = el('known-host-panel-host', `[data-record-id="${hostId}"]`);
    await host.waitForExist();
    await host.click();
    await waitFor('hosts-screen');
    await waitFor('host-panel', `[data-record-id="${hostId}"]`);
    await el('host-panel-collapse').click();
    await waitForAbsent('host-panel');
  });

  it('searches, switches the view and sorts, and keeps view and sort after a restart', async () => {
    seeded = await keygen('ed25519', { comment: 'seeded' });
    await seedKnownHost({ host: '192.0.2.50', key_type: seeded.keyType, public_key: seeded.publicKeyBase64 });
    await openKnownHosts();
    const ids = await waitForKnownHostCount(2);
    seededId = ids.find((id) => id !== tofuId) as string;
    expect(await knownHostCardText(seededId, 'known-host-card-title')).toBe('192.0.2.50');

    await clearAndType('known-hosts-search-input', '192.0.2');
    await waitForKnownHostCount(1);
    expect(await knownHostIds()).toEqual([seededId]);
    // The search covers fingerprints too.
    await clearAndType('known-hosts-search-input', sshd.hostKeyFingerprint.slice(7, 20));
    await waitForKnownHostCount(1);
    expect(await knownHostIds()).toEqual([tofuId]);
    await clearAndType('known-hosts-search-input', 'nothing-matches-this');
    await waitFor('known-hosts-no-results');
    await el('known-hosts-search-clear').click();
    await waitForKnownHostCount(2);

    await el('known-hosts-view-toggle-list').click();
    await waitFor('known-hosts-screen', '[data-layout="list"]');
    await el('known-hosts-sort-button').click();
    await waitFor('known-hosts-sort-menu');
    await el('known-hosts-sort-created').click();
    await waitFor('known-hosts-screen', '[data-sort="created"]');
    await expect(el('known-hosts-sort-button')).toHaveAttribute('data-sort', 'created');

    await browser.pause(500);
    await restartAndUnlock(session.password);
    await openKnownHosts();
    await waitFor('known-hosts-screen', '[data-layout="list"][data-sort="created"]');
    await el('known-hosts-view-toggle-grid').click();
    await waitFor('known-hosts-screen', '[data-layout="grid"]');
  });

  it('copies the fingerprint and removes an entry from the context menu', async () => {
    await rightClick(knownHostCard(seededId));
    await waitFor('known-host-context-menu');
    await el('known-host-menu-copy-fingerprint').click();
    await browser.waitUntil(async () => (await clipboardText()) === seeded.fingerprint, {
      timeout: 5_000,
      timeoutMsg: 'the context menu did not copy the fingerprint',
    });

    await rightClick(knownHostCard(seededId));
    await waitFor('known-host-context-menu');
    await el('known-host-menu-remove').click();
    await waitFor('known-host-remove-dialog');
    await el('known-host-remove-dialog-confirm').click();
    await waitForAbsent('known-host-card', `[data-record-id="${seededId}"]`);
    await waitForKnownHostCount(1);
  });

  it('removes an entry from the details panel after confirmation', async () => {
    // The toasts of the previous test sit at the bottom right, over the
    // panel's Remove button, until they time out (5 s).
    await waitForAbsent('known-hosts-removed-toast');
    await waitForAbsent('known-hosts-fingerprint-copied-toast');
    await knownHostCard(tofuId).click();
    await waitFor('known-host-panel', `[data-record-id="${tofuId}"]`);
    await el('known-host-panel-remove').click();
    await waitFor('known-host-remove-dialog');
    await el('known-host-remove-dialog-cancel').click();
    await waitForAbsent('known-host-remove-dialog');
    await expect(knownHostCard(tofuId)).toBeExisting();

    await el('known-host-panel-remove').click();
    await waitFor('known-host-remove-dialog');
    await el('known-host-remove-dialog-confirm').click();
    await waitForAbsent('known-host-card', `[data-record-id="${tofuId}"]`);
    await waitForAbsent('known-host-panel');
    await waitFor('known-hosts-empty');
  });

  it('shows a hashed entry as "Hashed entry" and matches it on connect without a question', async () => {
    await seedKnownHost({
      host: hashedHost('127.0.0.1', sshd.port),
      key_type: sshd.hostKeyType,
      public_key: sshd.hostKeyBase64,
    });
    const [hashedId] = await waitForKnownHostCount(1);
    await expect(knownHostCard(hashedId)).toHaveAttribute('data-hashed', 'true');
    const title = await knownHostCardText(hashedId, 'known-host-card-title');
    expect(title).not.toContain('127.0.0.1');
    expect(title.length).toBeGreaterThan(0);
    expect(await knownHostCardText(hashedId, 'known-host-card-subtitle')).toContain(sshd.hostKeyType);

    await openHostsScreen();
    const tab = await connectHost(hostId);
    // Connected without a host key question.
    let asked = false;
    await pollUntil(
      async () => {
        asked = asked || (await existsInPane('host-key-dialog'));
        return asked || (await paneState()) === 'connected';
      },
      { timeout: 20_000, message: () => 'the session did not connect' },
    );
    expect(asked).toBe(false);
    await waitForShell(user.name);
    await runCommand('exit');
    await waitForTabClosed(tab as string);
  });

  it('marks a revoked entry with the revoked badge', async () => {
    const revoked = await keygen('ed25519', { comment: 'revoked' });
    await seedKnownHost({ host: '198.51.100.9', key_type: revoked.keyType, public_key: revoked.publicKeyBase64, revoked: true });
    await activateVaults();
    await openKnownHosts();
    const ids = await waitForKnownHostCount(2);
    const revokedId = (
      await Promise.all(ids.map(async (id) => ((await knownHostCard(id).getAttribute('data-revoked')) === 'true' ? id : null)))
    ).find((id) => id !== null) as string;
    expect(revokedId).toBeDefined();
    await expect(
      $(`${tid('known-host-card', `[data-record-id="${revokedId}"]`)} ${tid('known-host-card-revoked-badge')}`),
    ).toBeExisting();
    expect(await knownHostCardText(revokedId, 'known-host-card-title')).toBe('198.51.100.9');
  });

  it('does not render Import and Export (stage 3)', async () => {
    await expectAbsent(HIDDEN);
  });
});
