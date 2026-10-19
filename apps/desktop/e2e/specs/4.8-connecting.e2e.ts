// UI 4.8 Connecting and authentication: the connection screen (host name,
// address, step, status, Cancel); the host key step; prompts for the
// username, the password (wrong value -> error in the same form, "Save" off
// by default and absent for a quick connect), keyboard-interactive questions
// one at a time with the server texts framed, and the key passphrase; the
// failure state with the reason, Retry and Close, without automatic attempts.

import {
  answerPrompt,
  clickInPane,
  connectHost,
  createSshHost,
  existsInPane,
  hostKey,
  keyToLink,
  paneAttr,
  paneState,
  paneText,
  pollUntil,
  quickConnect,
  quickTarget,
  restartAndUnlock,
  runCommand,
  startFresh,
  waitForShell,
  waitForTabClosed,
  waitInPane,
  type Session,
} from '../helpers/index.ts';
import { generatedSecret, keygen, startSshd, stopAllSshd, type GeneratedKey, type Sshd, type SshdUser } from '../support/sshd.ts';

async function expectInPane(ids: string[]): Promise<void> {
  const missing: string[] = [];
  for (const id of ids) {
    if (!(await existsInPane(id))) {
      missing.push(id);
    }
  }
  expect(missing).toEqual([]);
}

async function isChecked(id: string): Promise<boolean> {
  const checked = await paneAttr(id, 'aria-checked');
  if (checked !== null) {
    return checked === 'true';
  }
  return (await waitInPane(id)).isSelected();
}

/** Ends the active session with `exit` and waits for its tab to close. */
async function exitSession(tab: string | null): Promise<void> {
  await runCommand('exit');
  await waitForTabClosed(tab as string);
}

describe('UI 4.8 Connecting and authentication', () => {
  let session: Session;
  let pw: Sshd;
  let pwUser: SshdUser;
  let ki: Sshd;
  let kiUser: SshdUser;
  let keyServer: Sshd;
  let key: GeneratedKey;

  before(async () => {
    session = await startFresh();
    pw = await startSshd({ users: [{ name: 'pwuser' }] });
    pwUser = pw.users[0];
    ki = await startSshd({
      users: [
        {
          name: 'kiuser',
          keyboardInteractive: [
            { prompt: 'Verification code: ', echo: false },
            { prompt: 'Favorite color: ', echo: true },
          ],
          kiName: 'shellpp-test 2FA',
          kiInstruction: 'Answer both questions',
        },
      ],
    });
    kiUser = ki.users[0];
    key = await keygen('ed25519', { passphrase: generatedSecret('Pass-') });
    keyServer = await startSshd({ users: [{ name: 'keyuser', publicKeys: [key.publicKey] }] });
  });

  after(async () => {
    await stopAllSshd();
  });

  let connectHostId: string;

  it('shows the host, the address and the host key step; Cancel leads to the failure state', async () => {
    const hostId = await createSshHost(pw, pwUser, { label: 'Connect host' });
    connectHostId = hostId;
    const tab = await connectHost(hostId);
    await waitInPane('connecting-view');
    await waitInPane('host-key-dialog');
    expect(await paneText('connecting-title')).toBe('Connect host');
    const address = await paneText('connecting-address');
    expect(address).toContain(pw.address);
    expect(address).toContain(String(pw.port));
    expect(await paneAttr('connecting-view', 'data-step')).toBe('host_key');
    expect((await paneText('connecting-step')).length).toBeGreaterThan(0);
    await expectInPane(['connecting-status']);
    // At the host key step the dialog's Cancel is the only Cancel (UI 10.2).
    expect(await existsInPane('connecting-cancel')).toBe(false);

    await clickInPane('host-key-dialog-cancel');
    await waitInPane('connect-failed');
    expect(await paneAttr('connect-failed', 'data-code')).toBe('ssh_error');
    expect(await paneAttr('connect-failed', 'data-reason')).toBe('host_key_rejected');
    await expectInPane(['connect-failed-reason', 'connect-failed-retry', 'connect-failed-close']);
    // No automatic attempt after a failed first connection.
    await browser.pause(2_500);
    expect(await paneState()).toBe('failed');

    await clickInPane('connect-failed-retry');
    await hostKey('accept');
    await waitForShell(pwUser.name);
    await exitSession(tab);
  });

  it('cancels an attempt in progress with Cancel', async () => {
    // The server accepts the connection but never answers: the attempt hangs.
    await pw.setHolding(true);
    const tab = await connectHost(connectHostId);
    await waitInPane('connecting-cancel');
    expect(await paneAttr('connecting-view', 'data-state')).toBe('connecting');
    await clickInPane('connecting-cancel');
    await waitInPane('connect-failed');
    expect(await paneAttr('connect-failed', 'data-code')).toBe('cancelled');
    // No automatic attempt after a cancelled first connection.
    await browser.pause(2_500);
    expect(await paneState()).toBe('failed');
    await pw.setHolding(false);
    await pw.drop();
    await clickInPane('connect-failed-close');
    await waitForTabClosed(tab as string);
  });

  it('asks for the username and the password of a quick connect, without "Save"', async () => {
    // The server's key was saved above.
    const tab = await quickConnect(quickTarget(pw));
    await waitInPane('session-prompt', '[data-kind="username"]');
    await answerPrompt(pwUser.name, { kind: 'username' });
    await waitInPane('session-prompt', '[data-kind="password"]');
    expect(await existsInPane('session-prompt-save')).toBe(false);
    expect(await paneAttr('session-prompt-input', 'type')).toBe('password');

    // A wrong password: the error shows in the same form.
    await answerPrompt(generatedSecret('Wrong-'), { kind: 'password' });
    await waitInPane('session-prompt-input-error');
    await waitInPane('session-prompt', '[data-kind="password"]');
    await answerPrompt(pwUser.password as string, { kind: 'password' });
    await waitForShell(pwUser.name);
    await exitSession(tab);
  });

  it('saves the password with "Save" for the next connections', async () => {
    const hostId = await createSshHost(pw, pwUser, { label: 'Save host', savePassword: false });
    const tab = await connectHost(hostId);
    await waitInPane('session-prompt', '[data-kind="password"]');
    await waitInPane('session-prompt-save');
    // Off by default.
    expect(await isChecked('session-prompt-save')).toBe(false);
    await answerPrompt(pwUser.password as string, { kind: 'password', save: true });
    await waitForShell(pwUser.name);
    await exitSession(tab);

    await restartAndUnlock(session.password);
    const again = await connectHost(hostId);
    // No prompt this time.
    await waitForShell(pwUser.name);
    await exitSession(again);
  });

  it('asks keyboard-interactive questions one at a time, with the server texts framed', async () => {
    const tab = await quickConnect(quickTarget(ki, kiUser.name));
    await hostKey('accept');
    await waitInPane('session-prompt', '[data-kind="keyboard_interactive"]');
    const server = await paneText('session-prompt-server-text');
    expect(server).toContain('shellpp-test 2FA');
    expect(server).toContain('Answer both questions');
    expect(await paneText('session-prompt-label')).toContain('Verification code');
    // Not echoed: masked.
    expect(await paneAttr('session-prompt-input', 'type')).toBe('password');
    const progress = await paneText('session-prompt-progress');
    expect(progress).toContain('1');
    expect(progress).toContain('2');
    expect(await existsInPane('session-prompt-save')).toBe(false);

    await answerPrompt(kiUser.kiAnswers![0]);
    await pollUntil(async () => (await paneText('session-prompt-label')).includes('Favorite color'), {
      timeout: 10_000,
      message: () => 'the second question did not appear',
    });
    // Echoed: shown in clear.
    expect(await paneAttr('session-prompt-input', 'type')).not.toBe('password');
    await answerPrompt(kiUser.kiAnswers![1]);
    await waitForShell(kiUser.name);
    await exitSession(tab);
  });

  it('asks for the passphrase of an encrypted key that is not saved', async () => {
    const hostId = await createSshHost(keyServer, keyServer.users[0], {
      label: 'Key host',
      key: keyToLink(key, { savePassphrase: false, label: 'Encrypted e2e key' }),
    });
    const tab = await connectHost(hostId);
    await hostKey('accept');
    await waitInPane('session-prompt', '[data-kind="passphrase"]');
    await waitInPane('session-prompt-save');
    expect(await isChecked('session-prompt-save')).toBe(false);

    await answerPrompt(generatedSecret('Wrong-'), { kind: 'passphrase' });
    await waitInPane('session-prompt-input-error');
    await answerPrompt(key.passphrase as string, { kind: 'passphrase' });
    await waitForShell('keyuser');
    await exitSession(tab);
  });

  it('fails with the reason on a closed port and connects with Retry once the server listens', async () => {
    const refused = await startSshd({ users: [{ name: 'refuser' }] });
    const hostId = await createSshHost(refused, refused.users[0], { label: 'Refused host' });
    await refused.setAccepting(false);

    const tab = await connectHost(hostId);
    await waitInPane('connect-failed', '', 30_000);
    expect(await paneAttr('connect-failed', 'data-code')).toBe('ssh_error');
    expect(await paneAttr('connect-failed', 'data-reason')).toBe('refused');
    await expectInPane(['connect-failed-reason', 'connect-failed-retry', 'connect-failed-close']);
    await browser.pause(3_000);
    expect(await paneState()).toBe('failed');

    await refused.setAccepting(true);
    await clickInPane('connect-failed-retry');
    await hostKey('accept');
    await waitForShell('refuser');
    await exitSession(tab);

    // Close removes the tab.
    await refused.setAccepting(false);
    const second = await connectHost(hostId);
    await waitInPane('connect-failed', '', 30_000);
    await clickInPane('connect-failed-close');
    await waitForTabClosed(second as string);
    await refused.setAccepting(true);
  });

  it('ends as cancelled when the password prompt after a wrong saved password is cancelled', async () => {
    const hostId = await createSshHost(pw, pwUser, { label: 'Wrong password host', password: generatedSecret('Wrong-') });
    const tab = await connectHost(hostId);
    await waitInPane('session-prompt', '[data-kind="password"]');
    await clickInPane('session-prompt-cancel');
    await waitInPane('connect-failed');
    expect(await paneAttr('connect-failed', 'data-code')).toBe('cancelled');
    await clickInPane('connect-failed-close');
    await waitForTabClosed(tab as string);
  });
});
