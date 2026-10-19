// UI 10.2 First connection and key change: the first connection asks with
// address, key type and fingerprint ("Accept and save" is the default,
// "Connect once" stores nothing, "Cancel" fails); a changed key blocks the
// connection with a danger warning, the old and new fingerprints and only
// "Replace saved key" to continue (no "don't ask again"); a revoked key
// blocks the connection with no way to continue.

import {
  activateVaults,
  clickInPane,
  connectHost,
  createSshHost,
  displayAddress,
  el,
  existsInPane,
  hostKey,
  knownHostCard,
  openHostsScreen,
  openKnownHosts,
  paneAttr,
  paneSelector,
  paneState,
  paneText,
  pollUntil,
  runCommand,
  seedKnownHost,
  selectedTab,
  startFresh,
  textAt,
  tid,
  waitAbsentInPane,
  waitFor,
  waitForActivePane,
  waitForFocusAt,
  waitForKnownHostCount,
  waitForShell,
  waitForTabClosed,
  waitInPane,
} from '../helpers/index.ts';
import { startSshd, stopAllSshd, type Sshd, type SshdUser } from '../support/sshd.ts';

async function expectInPane(ids: string[], present = true): Promise<void> {
  const wrong: string[] = [];
  for (const id of ids) {
    if ((await existsInPane(id)) !== present) {
      wrong.push(id);
    }
  }
  expect(wrong).toEqual([]);
}

/** Connects without any host key question (waits for the shell, fails if one appears). */
async function connectsSilently(hostId: string, user: string): Promise<string> {
  const tab = await connectHost(hostId);
  let asked = false;
  await pollUntil(
    async () => {
      asked = asked || (await existsInPane('host-key-dialog'));
      return asked || (await paneState()) === 'connected';
    },
    { timeout: 20_000, message: () => 'the session did not connect' },
  );
  expect(asked).toBe(false);
  await waitForShell(user);
  return tab as string;
}

async function exitSession(tab: string | null): Promise<void> {
  await runCommand('exit');
  await waitForTabClosed(tab as string);
}

describe('UI 10.2 First connection and key change', () => {
  let sshd: Sshd;
  let user: SshdUser;
  let hostId: string;
  let originalFingerprint: string;

  before(async () => {
    await startFresh();
    sshd = await startSshd();
    user = sshd.users[0];
    hostId = await createSshHost(sshd, user, { label: 'Key change host' });
    originalFingerprint = sshd.hostKeyFingerprint;
  });

  after(async () => {
    await stopAllSshd();
  });

  it('asks on the first connection with the address, the key type and the fingerprint', async () => {
    const tab = await connectHost(hostId);
    await waitInPane('host-key-dialog', '[data-kind="unknown"]');
    expect(await paneText('host-key-dialog-address')).toContain(displayAddress(sshd));
    expect(await paneText('host-key-dialog-key-type')).toContain(sshd.hostKeyType);
    expect(await paneText('host-key-dialog-fingerprint')).toBe(sshd.hostKeyFingerprint);
    await expectInPane(['host-key-dialog-accept', 'host-key-dialog-once', 'host-key-dialog-cancel']);
    await expectInPane(['host-key-dialog-replace', 'host-key-dialog-warning'], false);
    // "Accept and save" is the default button.
    await waitForFocusAt(`${paneSelector(await waitForActivePane())} ${tid('host-key-dialog-accept')}`);

    // Cancel: the connection fails, nothing is stored.
    await hostKey('cancel');
    await waitInPane('connect-failed');
    await clickInPane('connect-failed-close');
    await waitForTabClosed(tab as string);
  });

  it('connects once without storing the key and asks again next time', async () => {
    const tab = await connectHost(hostId);
    await hostKey('once');
    await waitForShell(user.name);
    await exitSession(tab);
    await openKnownHosts();
    await waitFor('known-hosts-empty');

    await openHostsScreen();
    const again = await connectHost(hostId);
    await waitInPane('host-key-dialog', '[data-kind="unknown"]');
    // Accept and save this time.
    await hostKey('accept');
    await waitForShell(user.name);
    await exitSession(again);
  });

  it('stores an accepted key and does not ask again', async () => {
    await openKnownHosts();
    const [id] = await waitForKnownHostCount(1);
    await knownHostCard(id).click();
    await waitFor('known-host-panel', `[data-record-id="${id}"]`);
    expect(await textAt(`${tid('known-host-panel')} ${tid('known-host-panel-fingerprint')}`)).toBe(originalFingerprint);
    await el('known-host-panel-collapse').click();

    await openHostsScreen();
    const tab = await connectsSilently(hostId, user.name);
    await exitSession(tab);
  });

  it('blocks a changed key with a danger warning and both fingerprints', async () => {
    const rotated = await sshd.rotateHostKey();
    expect(rotated.fingerprint).not.toBe(originalFingerprint);
    const tab = await connectHost(hostId);
    await waitInPane('host-key-dialog', '[data-kind="changed"]');
    await expectInPane(['host-key-dialog-warning', 'host-key-dialog-replace', 'host-key-dialog-cancel']);
    await expectInPane(['host-key-dialog-accept', 'host-key-dialog-once'], false);
    expect(await paneText('host-key-dialog-old-fingerprint')).toBe(originalFingerprint);
    expect(await paneText('host-key-dialog-fingerprint')).toBe(rotated.fingerprint);
    expect(await paneText('host-key-dialog-old-key-type')).toContain(sshd.hostKeyType);
    expect(await paneText('host-key-dialog-key-type')).toContain(rotated.keyType);
    // No "don't ask again" checkbox.
    const checkboxes = await browser.execute(
      (selector: string) => document.querySelectorAll(`${selector} input[type="checkbox"], ${selector} [role="checkbox"]`).length,
      `${paneSelector(await waitForActivePane())} ${tid('host-key-dialog')}`,
    );
    expect(checkboxes).toBe(0);

    await hostKey('cancel');
    await waitInPane('connect-failed');
    expect(await paneState()).toBe('failed');
    expect(tab).not.toBeNull();
  });

  it('continues only with "Replace saved key", which stores the new key', async () => {
    await clickInPane('connect-failed-retry');
    await waitInPane('host-key-dialog', '[data-kind="changed"]');
    await hostKey('replace');
    await waitForShell(user.name);
    await waitAbsentInPane('connecting-view');
    const tab = await selectedTab();
    expect(tab).not.toBe('vaults');
    await exitSession(tab);

    await openKnownHosts();
    const [id] = await waitForKnownHostCount(1);
    await knownHostCard(id).click();
    await waitFor('known-host-panel', `[data-record-id="${id}"]`);
    expect(await textAt(`${tid('known-host-panel')} ${tid('known-host-panel-fingerprint')}`)).toBe(sshd.hostKeyFingerprint);
    await el('known-host-panel-collapse').click();
  });

  it('blocks a revoked key without a way to continue', async () => {
    // A revoked entry with the server's key, under any host pattern.
    await seedKnownHost({ host: '203.0.113.5', key_type: sshd.hostKeyType, public_key: sshd.hostKeyBase64, revoked: true });
    await activateVaults();
    await openHostsScreen();
    const tab = await connectHost(hostId);
    await waitInPane('host-key-revoked', '', 20_000);
    expect(await paneText('host-key-revoked-fingerprint')).toBe(sshd.hostKeyFingerprint);
    expect(await paneText('host-key-revoked-key-type')).toContain(sshd.hostKeyType);
    await waitInPane('connect-failed');
    expect(await paneAttr('connect-failed', 'data-code')).toBe('host_key_revoked');
    await expectInPane(['connect-failed-close']);
    await expectInPane(['connect-failed-retry', 'host-key-dialog-replace', 'host-key-dialog-accept', 'host-key-dialog-once'], false);
    await clickInPane('connect-failed-close');
    await waitForTabClosed(tab as string);
  });
});
