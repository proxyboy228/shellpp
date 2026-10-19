// The russh test SSH server for the e2e specs (docs/architecture.md, section
// 9.3; stage 2 plan, sections 4.1 and 8.1). Every spec starts its own
// servers with `startSshd()` and stops them in `after` (the WebdriverIO
// `afterSession` hook and a process exit hook stop leftovers).
//
// The server is the `shellpp-test-sshd` binary built by scripts/e2e-build.mjs
// (`sshdPath()`), driven through its CLI contract (tests/sshd/src/main.rs):
//
//   shellpp-test-sshd serve --config <file.json>
//     stdout: {"event":"ready","port":N,"host_key_type":..,"host_key_fingerprint":..,"host_key_base64":..}
//     stdin:  rotate-host-key | drop | accept off|on | events | quit, one JSON answer line each
//   shellpp-test-sshd keygen --type <t> --out <path> [--passphrase-env VAR] [--comment c]
//     stdout: {"public_key":..,"fingerprint":..}
//
// Secrets (user passwords, keyboard-interactive answers, key passphrases)
// are generated per call with a CSPRNG (CONTRIBUTING, "Secrets and test
// data"); the config file and generated keys live in temporary directories
// that `stop()` and the exit hook delete. The server listens on 127.0.0.1 on
// a free port and logs to stderr without secrets; stderr goes to
// target/e2e-logs/<worker>-sshd-<n>.log.

import { spawn, type ChildProcess } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import readline from 'node:readline';
import { logDir, sshdPath } from './paths.ts';

/** Key types of the test server (`--type` of `keygen`, `host_key.generate` of `serve`). */
export type SshdKeyType = 'ed25519' | 'ecdsa-p256' | 'rsa-3072';

/** One keyboard-interactive question; all questions of a user come in one round, in order. */
export interface KiQuestionSpec {
  prompt: string;
  /** `false`: the client must mask the answer. */
  echo: boolean;
  /** Generated when omitted (answers are never fixed in specs). */
  answer?: string;
}

export interface SshdUserSpec {
  /** Default `test` for the first user, `test<n>` for the others. */
  name?: string;
  /**
   * `true`: a generated password; a string: this password (a spec generates
   * it, for example with `newPassword()`); `false`: no password
   * authentication. Default: `true` unless public keys or keyboard-interactive
   * questions are given.
   */
  password?: boolean | string;
  /** OpenSSH public key lines accepted for public key authentication. */
  publicKeys?: string[];
  keyboardInteractive?: KiQuestionSpec[];
  /** keyboard-interactive name and instruction (server texts shown framed, UI 4.8). */
  kiName?: string;
  kiInstruction?: string;
}

/** A configured account with its generated secrets. */
export interface SshdUser {
  name: string;
  password?: string;
  /** Answers of the keyboard-interactive questions, in order. */
  kiAnswers?: string[];
}

export interface SshdOptions {
  /** Default: one user `test` with a generated password. */
  users?: SshdUserSpec[];
  /** Comment of the identification string, e.g. `Ubuntu-3ubuntu13` (OS detection from the banner). */
  bannerComment?: string;
  /** Answer of every exec request (the OS detection script); exec is refused when absent. */
  exec?: { stdout: string; exit_status?: number };
  /** MOTD printed when the shell starts (default `Welcome to shellpp-test-sshd`). */
  motd?: string;
  /** SSH authentication banner text. */
  authBanner?: string;
  /** Host key type (default `ed25519`). */
  hostKeyType?: SshdKeyType;
}

/** A server event (`ServerEvent` of the test server, JSON tag `kind`). */
export interface SshdEvent {
  kind: string;
  [field: string]: unknown;
}

export interface Sshd {
  /** Always `127.0.0.1`. */
  readonly address: string;
  readonly port: number;
  /** SSH key type name, e.g. `ssh-ed25519` (updated by `rotateHostKey`). */
  readonly hostKeyType: string;
  /** `SHA256:...` as OpenSSH prints it (updated by `rotateHostKey`). */
  readonly hostKeyFingerprint: string;
  /** Base64 key blob as in a known_hosts line (updated by `rotateHostKey`). */
  readonly hostKeyBase64: string;
  readonly users: SshdUser[];
  /** Drops every connection and switches to a new host key of the same type on the same port. */
  rotateHostKey(): Promise<{ fingerprint: string; base64: string; keyType: string }>;
  /** Closes every TCP connection without an SSH disconnect or exit status; returns the count. */
  drop(): Promise<number>;
  /** `false`: the port refuses connections; `true`: listens again on the same port. */
  setAccepting(on: boolean): Promise<void>;
  /** `true`: new connections are accepted and never answered (a connection attempt hangs); `drop()` ends them. */
  setHolding(on: boolean): Promise<void>;
  /** Events recorded so far (auth attempts, PTY, env, exec, ...). */
  events(): Promise<SshdEvent[]>;
  /** Quits the server (kills it after 5 s) and deletes its temporary files. */
  stop(): Promise<void>;
}

export interface GeneratedKey {
  /** Private key file (OpenSSH format); its public key is `<path>.pub`. */
  path: string;
  /** OpenSSH private key text, encrypted when a passphrase was given. */
  privateKey: string;
  /** One OpenSSH public key line, `<type> <base64> <comment>`. */
  publicKey: string;
  /** Key type name of the public key line, e.g. `ssh-ed25519`. */
  keyType: string;
  /** Base64 key blob (second field of the public key line). */
  publicKeyBase64: string;
  /** `SHA256:...` */
  fingerprint: string;
  /** The passphrase the key is encrypted with, if any. */
  passphrase?: string;
}

const READY_TIMEOUT_MS = 30_000;
const COMMAND_TIMEOUT_MS = 15_000;
const STOP_TIMEOUT_MS = 5_000;

/** A generated password: 24 characters from a CSPRNG. */
export function generatedSecret(prefix = 'Pw-'): string {
  return `${prefix}${crypto.randomBytes(18).toString('base64url')}`;
}

/** A generated keyboard-interactive answer (hex, never fixed). */
function generatedAnswer(): string {
  return crypto.randomBytes(9).toString('hex');
}

// ---------------------------------------------------------------------------
// Bookkeeping: running servers and temporary directories of this worker
// ---------------------------------------------------------------------------

const runningServers = new Set<SshdProcess>();
const tempDirs = new Set<string>();
let serverCount = 0;
let keysDirPath: string | undefined;
let keyCount = 0;

function workerName(): string {
  const cid = process.env.WDIO_WORKER_ID ?? `pid${process.pid}`;
  return cid.replace(/[^A-Za-z0-9._-]+/g, '_');
}

process.on('exit', () => {
  for (const server of runningServers) {
    server.killNow();
  }
  for (const dir of tempDirs) {
    try {
      fs.rmSync(dir, { recursive: true, force: true });
    } catch {
      // Best effort at exit.
    }
  }
});

function ensureBinary(): string {
  const binary = sshdPath();
  if (!fs.existsSync(binary)) {
    throw new Error(
      `The test SSH server binary is missing: ${binary}\n` +
        'Build it with "pnpm --filter @shellpp/desktop e2e:build" (or "-- --sshd-only") or set SHELLPP_E2E_SSHD.',
    );
  }
  return binary;
}

async function removeDir(dir: string): Promise<void> {
  tempDirs.delete(dir);
  try {
    await fsp.rm(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  } catch (error) {
    console.warn(`[e2e] Could not delete ${dir}: ${(error as Error).message}`);
  }
}

function tail(file: string, lines = 20): string {
  try {
    return fs.readFileSync(file, 'utf8').split(/\r?\n/).slice(-lines).join('\n');
  } catch {
    return '(no log)';
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ---------------------------------------------------------------------------
// The `serve` process and its line protocol
// ---------------------------------------------------------------------------

interface Waiter {
  resolve(line: Record<string, unknown>): void;
  reject(error: Error): void;
}

class SshdProcess {
  readonly child: ChildProcess;
  readonly dir: string;
  readonly logFile: string;
  private readonly log: fs.WriteStream;
  private readonly waiters: Waiter[] = [];
  private exitError: Error | undefined;
  private readonly exited: Promise<void>;

  constructor(child: ChildProcess, dir: string, logFile: string, log: fs.WriteStream) {
    this.child = child;
    this.dir = dir;
    this.logFile = logFile;
    this.log = log;
    const lines = readline.createInterface({ input: child.stdout! });
    lines.on('line', (line) => this.onLine(line));
    child.stderr?.pipe(log, { end: false });
    this.exited = new Promise((resolve) => {
      child.once('exit', (code, signal) => {
        this.exitError = new Error(
          `shellpp-test-sshd exited (code ${String(code)}, signal ${String(signal)}).\n${tail(logFile)}`,
        );
        for (const waiter of this.waiters.splice(0)) {
          waiter.reject(this.exitError);
        }
        resolve();
      });
      child.once('error', (error) => {
        this.exitError = new Error(`shellpp-test-sshd could not run: ${error.message}`);
        for (const waiter of this.waiters.splice(0)) {
          waiter.reject(this.exitError);
        }
        resolve();
      });
    });
  }

  get running(): boolean {
    return this.child.exitCode === null && this.child.signalCode === null && this.exitError === undefined;
  }

  private onLine(text: string): void {
    const trimmed = text.trim();
    if (trimmed === '') {
      return;
    }
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(trimmed) as Record<string, unknown>;
    } catch {
      // Not a protocol line; keep it in the log for debugging.
      this.log.write(`[stdout] ${trimmed}\n`);
      return;
    }
    const waiter = this.waiters.shift();
    if (waiter === undefined) {
      this.log.write(`[unexpected] ${trimmed}\n`);
      return;
    }
    waiter.resolve(parsed);
  }

  /** Waits for the next JSON line (the answer to the last command, or the ready line). */
  next(expectedEvent: string, timeout: number, what: string): Promise<Record<string, unknown>> {
    if (this.exitError) {
      return Promise.reject(this.exitError);
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const index = this.waiters.indexOf(waiter);
        if (index >= 0) {
          this.waiters.splice(index, 1);
        }
        reject(new Error(`shellpp-test-sshd did not answer "${what}" within ${timeout} ms.\n${tail(this.logFile)}`));
      }, timeout);
      const waiter: Waiter = {
        resolve: (line) => {
          clearTimeout(timer);
          if (line.event === expectedEvent) {
            resolve(line);
          } else {
            reject(new Error(`shellpp-test-sshd answered "${what}" with ${JSON.stringify(line)}`));
          }
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      };
      this.waiters.push(waiter);
    });
  }

  /** Sends one stdin command and waits for its answer line. */
  async command(command: string, expectedEvent: string, timeout = COMMAND_TIMEOUT_MS): Promise<Record<string, unknown>> {
    if (!this.running) {
      throw this.exitError ?? new Error('shellpp-test-sshd is not running');
    }
    const answer = this.next(expectedEvent, timeout, command);
    this.child.stdin!.write(`${command}\n`);
    return answer;
  }

  /** Kills the process at once (exit hook). */
  killNow(): void {
    if (this.running) {
      try {
        this.child.kill();
      } catch {
        // Already gone.
      }
    }
  }

  /** `quit`, then a kill after 5 s; deletes the temporary directory. */
  async stop(): Promise<void> {
    runningServers.delete(this);
    if (this.running) {
      // The answer to `quit` is `{"event":"bye"}`; the exit is what counts.
      void this.next('bye', STOP_TIMEOUT_MS, 'quit').catch(() => undefined);
      try {
        this.child.stdin!.write('quit\n');
        this.child.stdin!.end();
      } catch {
        // stdin already closed: the server exits on its own.
      }
      await Promise.race([this.exited, delay(STOP_TIMEOUT_MS)]);
      if (this.running) {
        this.child.kill();
        await Promise.race([this.exited, delay(STOP_TIMEOUT_MS)]);
      }
    }
    this.log.end();
    await removeDir(this.dir);
  }
}

function stringField(line: Record<string, unknown>, name: string): string {
  const value = line[name];
  if (typeof value !== 'string' || value === '') {
    throw new Error(`shellpp-test-sshd: "${name}" missing in ${JSON.stringify(line)}`);
  }
  return value;
}

/**
 * Starts a test server on 127.0.0.1 with a free port and a generated host
 * key. Users get generated passwords and answers unless the spec passes its
 * own (generated) values.
 */
export async function startSshd(options: SshdOptions = {}): Promise<Sshd> {
  const binary = ensureBinary();
  const specs = options.users ?? [{}];
  const users: SshdUser[] = [];
  const configUsers = specs.map((spec, index) => {
    const name = spec.name ?? (index === 0 ? 'test' : `test${index + 1}`);
    const keys = spec.publicKeys ?? [];
    const questions = spec.keyboardInteractive ?? [];
    const wantsPassword = spec.password ?? (keys.length === 0 && questions.length === 0);
    const password = typeof wantsPassword === 'string' ? wantsPassword : wantsPassword ? generatedSecret() : undefined;
    const answers = questions.map((question) => question.answer ?? generatedAnswer());
    users.push({
      name,
      ...(password === undefined ? {} : { password }),
      ...(questions.length === 0 ? {} : { kiAnswers: answers }),
    });
    return {
      name,
      ...(password === undefined ? {} : { password }),
      authorized_keys: keys,
      keyboard_interactive: questions.map((question, position) => ({
        prompt: question.prompt,
        echo: question.echo,
        answer: answers[position],
      })),
      ki_name: spec.kiName ?? '',
      ki_instruction: spec.kiInstruction ?? '',
    };
  });
  const config = {
    host_key: { generate: options.hostKeyType ?? 'ed25519' },
    users: configUsers,
    ...(options.bannerComment === undefined ? {} : { banner_comment: options.bannerComment }),
    ...(options.exec === undefined
      ? {}
      : { exec: { stdout: options.exec.stdout, exit_status: options.exec.exit_status ?? 0 } }),
    ...(options.motd === undefined ? {} : { motd: options.motd }),
    ...(options.authBanner === undefined ? {} : { auth_banner: options.authBanner }),
  };

  const dir = await fsp.mkdtemp(path.join(os.tmpdir(), 'shellpp-e2e-sshd-'));
  tempDirs.add(dir);
  const configFile = path.join(dir, 'server.json');
  await fsp.writeFile(configFile, `${JSON.stringify(config)}\n`, { encoding: 'utf8', mode: 0o600 });

  const logs = logDir();
  await fsp.mkdir(logs, { recursive: true });
  serverCount += 1;
  const logFile = path.join(logs, `${workerName()}-sshd-${serverCount}.log`);
  const log = fs.createWriteStream(logFile, { flags: 'w' });

  const child = spawn(binary, ['serve', '--config', configFile], {
    cwd: dir,
    env: { ...process.env, RUST_BACKTRACE: process.env.RUST_BACKTRACE ?? '1' },
    stdio: ['pipe', 'pipe', 'pipe'],
    windowsHide: true,
  });
  const server = new SshdProcess(child, dir, logFile, log);
  runningServers.add(server);

  let ready: Record<string, unknown>;
  try {
    ready = await server.next('ready', READY_TIMEOUT_MS, 'serve (ready line)');
  } catch (error) {
    await server.stop();
    throw error;
  }
  // The config holds the passwords: the server has read it, so delete it now.
  await fsp.rm(configFile, { force: true });

  const port = Number(ready.port);
  if (!Number.isInteger(port) || port <= 0) {
    await server.stop();
    throw new Error(`shellpp-test-sshd reported no port: ${JSON.stringify(ready)}`);
  }
  let hostKeyType = stringField(ready, 'host_key_type');
  let hostKeyFingerprint = stringField(ready, 'host_key_fingerprint');
  let hostKeyBase64 = stringField(ready, 'host_key_base64');

  return {
    address: '127.0.0.1',
    port,
    get hostKeyType() {
      return hostKeyType;
    },
    get hostKeyFingerprint() {
      return hostKeyFingerprint;
    },
    get hostKeyBase64() {
      return hostKeyBase64;
    },
    users,
    async rotateHostKey() {
      const answer = await server.command('rotate-host-key', 'rotated');
      hostKeyType = stringField(answer, 'host_key_type');
      hostKeyFingerprint = stringField(answer, 'host_key_fingerprint');
      hostKeyBase64 = stringField(answer, 'host_key_base64');
      return { fingerprint: hostKeyFingerprint, base64: hostKeyBase64, keyType: hostKeyType };
    },
    async drop() {
      const answer = await server.command('drop', 'dropped');
      return Number(answer.count ?? 0);
    },
    async setAccepting(on: boolean) {
      const answer = await server.command(`accept ${on ? 'on' : 'off'}`, 'accepting');
      if (answer.value !== on) {
        throw new Error(`shellpp-test-sshd: accept ${on ? 'on' : 'off'} answered ${JSON.stringify(answer)}`);
      }
    },
    async setHolding(on: boolean) {
      const answer = await server.command(`hold ${on ? 'on' : 'off'}`, 'holding');
      if (answer.value !== on) {
        throw new Error(`shellpp-test-sshd: hold ${on ? 'on' : 'off'} answered ${JSON.stringify(answer)}`);
      }
    },
    async events() {
      const answer = await server.command('events', 'events');
      return Array.isArray(answer.items) ? (answer.items as SshdEvent[]) : [];
    },
    stop: () => server.stop(),
  };
}

/** Stops every server this worker started (WebdriverIO `afterSession`, spec `after` hooks). */
export async function stopAllSshd(): Promise<void> {
  await Promise.all([...runningServers].map((server) => server.stop()));
}

// ---------------------------------------------------------------------------
// keygen
// ---------------------------------------------------------------------------

function keysDir(): string {
  if (keysDirPath === undefined || !fs.existsSync(keysDirPath)) {
    keysDirPath = fs.mkdtempSync(path.join(os.tmpdir(), 'shellpp-e2e-keys-'));
    tempDirs.add(keysDirPath);
  }
  return keysDirPath;
}

/** Deletes the generated keys of this worker (also done at process exit). */
export async function removeGeneratedKeys(): Promise<void> {
  if (keysDirPath !== undefined) {
    const dir = keysDirPath;
    keysDirPath = undefined;
    await removeDir(dir);
  }
}

/**
 * Generates a client key with the test server's `keygen` (OpenSSH format,
 * encrypted when a passphrase is given). The passphrase reaches the binary
 * through an environment variable, never the command line.
 */
export async function keygen(
  type: SshdKeyType,
  options: { passphrase?: string; comment?: string } = {},
): Promise<GeneratedKey> {
  const binary = ensureBinary();
  keyCount += 1;
  const out = path.join(keysDir(), `id_${type.replace(/[^a-z0-9]+/g, '_')}_${keyCount}`);
  const args = ['keygen', '--type', type, '--out', out, '--comment', options.comment ?? 'shellpp-e2e'];
  const env: NodeJS.ProcessEnv = { ...process.env };
  if (options.passphrase !== undefined && options.passphrase !== '') {
    env.SHELLPP_E2E_KEY_PASSPHRASE = options.passphrase;
    args.push('--passphrase-env', 'SHELLPP_E2E_KEY_PASSPHRASE');
  }
  const output = await new Promise<string>((resolve, reject) => {
    const child = spawn(binary, args, { env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk: string) => {
      stderr += chunk;
    });
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`shellpp-test-sshd keygen --type ${type} did not finish within 60 s`));
    }, 60_000);
    child.once('error', (error) => {
      clearTimeout(timer);
      reject(new Error(`shellpp-test-sshd keygen could not run: ${error.message}`));
    });
    child.once('exit', (code) => {
      clearTimeout(timer);
      if (code === 0) {
        resolve(stdout);
      } else {
        reject(new Error(`shellpp-test-sshd keygen --type ${type} exited with code ${String(code)}:\n${stderr.slice(-2000)}`));
      }
    });
  });
  const line = output
    .split(/\r?\n/)
    .map((text) => text.trim())
    .filter((text) => text.startsWith('{'))
    .at(-1);
  if (line === undefined) {
    throw new Error(`shellpp-test-sshd keygen printed no JSON line: ${output.slice(0, 500)}`);
  }
  const parsed = JSON.parse(line) as Record<string, unknown>;
  const publicKey = stringField(parsed, 'public_key');
  const fingerprint = stringField(parsed, 'fingerprint');
  const [keyType = '', publicKeyBase64 = ''] = publicKey.split(/\s+/);
  const privateKey = await fsp.readFile(out, 'utf8');
  return {
    path: out,
    privateKey,
    publicKey,
    keyType,
    publicKeyBase64,
    fingerprint,
    ...(options.passphrase === undefined ? {} : { passphrase: options.passphrase }),
  };
}
