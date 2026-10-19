// Harness self-check (not a UI section spec): the driver starts the e2e
// binary with a fresh data directory, the page loads, and a restart keeps the
// directory; the test SSH server starts on a free loopback port and its
// switches (drop, accept off/on, host key rotation, events) and keygen work.
// Works against the skeleton app (without an app shell) and the full app.
// Run with:
//   pnpm --filter @shellpp/desktop e2e -- --spec e2e/smoke/harness.e2e.ts

import fs from 'node:fs';
import net from 'node:net';
import { exists, hashedHost } from '../helpers/index.ts';
import { generatedSecret, keygen, startSshd, stopAllSshd, type Sshd } from '../support/sshd.ts';

async function documentReady(): Promise<void> {
  await browser.waitUntil(
    async () => (await browser.execute(() => document.readyState)) === 'complete',
    { timeout: 30_000, timeoutMsg: 'the page did not finish loading' },
  );
}

/** Reads the SSH identification line of a server, or the connect error code. */
function identification(port: number): Promise<{ line?: string; error?: string }> {
  return new Promise((resolve) => {
    const socket = net.connect({ host: '127.0.0.1', port });
    let data = '';
    const done = (result: { line?: string; error?: string }) => {
      socket.destroy();
      resolve(result);
    };
    socket.setTimeout(10_000, () => done({ error: 'timeout' }));
    socket.on('data', (chunk) => {
      data += chunk.toString('latin1');
      const end = data.indexOf('\n');
      if (end >= 0) {
        done({ line: data.slice(0, end).trim() });
      }
    });
    socket.once('error', (error: NodeJS.ErrnoException) => done({ error: error.code ?? error.message }));
  });
}

/** True for a server that is still a skeleton (`todo!()` in tests/sshd). */
function notImplemented(error: unknown): boolean {
  return /not yet implemented|not implemented/i.test(error instanceof Error ? error.message : String(error));
}

describe('e2e harness', () => {
  let dataDir: string;

  it('starts the app with a fresh temporary data directory', async () => {
    dataDir = process.env.SHELLPP_E2E_DATA_DIR ?? '';
    expect(dataDir).not.toBe('');
    expect(fs.existsSync(dataDir)).toBe(true);
    const handles = await browser.getWindowHandles();
    expect(handles.length).toBeGreaterThan(0);
    await documentReady();
  });

  it('loads the bundled e2e frontend', async () => {
    const origin = await browser.execute(() => window.location.origin);
    // The custom protocol of a built Tauri app, never the Vite dev server.
    expect(origin).not.toContain('localhost:5173');
    if (await exists('app-root')) {
      const helper = await browser.execute(() => typeof window.__SHELLPP_E2E__);
      expect(helper).toBe('object');
      // Stage 2: the terminal probe registry is part of the helper.
      const probes = await browser.execute(() => Array.isArray(window.__SHELLPP_E2E__?.terminalPanes()));
      expect(probes).toBe(true);
    }
  });

  it('restarts the app in the same data directory', async () => {
    const before = browser.sessionId;
    await browser.reloadSession();
    expect(browser.sessionId).not.toBe(before);
    expect(process.env.SHELLPP_E2E_DATA_DIR).toBe(dataDir);
    await documentReady();
    if (await exists('app-root')) {
      // The real app writes its database into SHELLPP_DATA_DIR.
      expect(fs.readdirSync(dataDir).length).toBeGreaterThan(0);
    }
  });
});

describe('e2e harness: test SSH server', () => {
  let sshd: Sshd | undefined;

  before(async function () {
    try {
      sshd = await startSshd({ users: [{}, { name: 'second', keyboardInteractive: [{ prompt: 'Code: ', echo: false }] }] });
    } catch (error) {
      if (notImplemented(error)) {
        console.warn('[e2e] shellpp-test-sshd is not implemented yet (tests/sshd skeleton): skipping its self-check');
        this.skip();
      }
      throw error;
    }
  });

  after(async () => {
    await stopAllSshd();
  });

  it('listens on a free loopback port with a generated host key and generated secrets', async () => {
    const server = sshd as Sshd;
    expect(server.address).toBe('127.0.0.1');
    expect(server.port).toBeGreaterThan(0);
    expect(server.port).not.toBe(22);
    expect(server.hostKeyType).toBe('ssh-ed25519');
    expect(server.hostKeyFingerprint).toMatch(/^SHA256:[A-Za-z0-9+/]{43}$/);
    expect(server.hostKeyBase64.length).toBeGreaterThan(40);
    expect(server.users[0].name).toBe('test');
    expect(server.users[0].password?.length ?? 0).toBeGreaterThanOrEqual(20);
    expect(server.users[1].password).toBeUndefined();
    expect(server.users[1].kiAnswers?.[0]?.length ?? 0).toBeGreaterThan(8);
    const hello = await identification(server.port);
    expect(hello.line ?? hello.error).toMatch(/^SSH-2\.0-shellpp_test_sshd/);
  });

  it('refuses connections with accept off and listens again on the same port with accept on', async () => {
    const server = sshd as Sshd;
    await server.setAccepting(false);
    expect((await identification(server.port)).error).toBe('ECONNREFUSED');
    await server.setAccepting(true);
    expect((await identification(server.port)).line).toMatch(/^SSH-2\.0-/);
  });

  it('drops connections, rotates the host key on the same port and reports events', async () => {
    const server = sshd as Sshd;
    expect(await server.drop()).toBeGreaterThanOrEqual(0);
    const before = server.hostKeyFingerprint;
    const rotated = await server.rotateHostKey();
    expect(rotated.fingerprint).not.toBe(before);
    expect(server.hostKeyFingerprint).toBe(rotated.fingerprint);
    expect(rotated.keyType).toBe('ssh-ed25519');
    expect((await identification(server.port)).line).toMatch(/^SSH-2\.0-/);
    expect(Array.isArray(await server.events())).toBe(true);
  });

  it('generates plain and encrypted client keys', async () => {
    const plain = await keygen('ed25519', { comment: 'smoke' });
    expect(plain.publicKey).toMatch(/^ssh-ed25519 [A-Za-z0-9+/=]+ smoke$/);
    expect(plain.keyType).toBe('ssh-ed25519');
    expect(plain.fingerprint).toMatch(/^SHA256:/);
    expect(plain.privateKey).toContain('BEGIN OPENSSH PRIVATE KEY');
    expect(fs.existsSync(`${plain.path}.pub`)).toBe(true);
    const encrypted = await keygen('ecdsa-p256', { passphrase: generatedSecret('Pass-') });
    expect(encrypted.keyType).toBe('ecdsa-sha2-nistp256');
    expect(encrypted.privateKey).toContain('BEGIN OPENSSH PRIVATE KEY');
    expect(encrypted.privateKey).not.toBe(plain.privateKey);
  });

  it('hashes known_hosts names like OpenSSH', () => {
    const hashed = hashedHost('127.0.0.1', (sshd as Sshd).port);
    expect(hashed).toMatch(/^\|1\|[A-Za-z0-9+/]{27}=\|[A-Za-z0-9+/]{27}=$/);
  });

  it('stops the server and frees the port', async () => {
    const server = sshd as Sshd;
    await server.stop();
    expect((await identification(server.port)).error).toBe('ECONNREFUSED');
  });
});
