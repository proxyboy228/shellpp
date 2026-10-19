// Per-spec environment: a fresh temporary SHELLPP_DATA_DIR and a tauri-driver
// process that launches the app with it (docs/architecture.md, section 9.4).
// Called from the WebdriverIO `beforeSession` and `afterSession` hooks, which
// run once per spec file because every spec runs in its own worker.

import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { resolveNativeDriver, resolveTauriDriver } from './drivers.ts';
import { logDir } from './paths.ts';

export interface DriverSettings {
  /** tauri-driver port (WebdriverIO connects here). */
  port: number;
  /** Port of the native driver behind tauri-driver. */
  nativePort: number;
}

interface RunningDriver {
  process: ChildProcess;
  dataDir: string;
  logFile: string;
  log: fs.WriteStream;
}

let running: RunningDriver | undefined;

/** Fails fast when something (a stale tauri-driver) already listens on the port. */
async function assertPortFree(port: number): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const server = net.createServer();
    server.once('error', (error: NodeJS.ErrnoException) => {
      reject(
        new Error(
          error.code === 'EADDRINUSE'
            ? `Port ${port} is in use. Stop the stale tauri-driver or msedgedriver process, ` +
                'or set SHELLPP_E2E_DRIVER_PORT / SHELLPP_E2E_NATIVE_PORT.'
            : `Cannot check port ${port}: ${error.message}`,
        ),
      );
    });
    server.listen(port, '127.0.0.1', () => {
      server.close(() => resolve());
    });
  });
}

function canConnect(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = net.connect({ host: '127.0.0.1', port });
    socket.once('connect', () => {
      socket.destroy();
      resolve(true);
    });
    socket.once('error', () => {
      socket.destroy();
      resolve(false);
    });
  });
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function tail(file: string, lines = 30): string {
  try {
    return fs.readFileSync(file, 'utf8').split(/\r?\n/).slice(-lines).join('\n');
  } catch {
    return '(no log)';
  }
}

function safeName(text: string): string {
  return text.replace(/[^A-Za-z0-9._-]+/g, '_').slice(0, 80);
}

/** Resolves the driver binaries once per process (the launcher passes them on in env). */
export async function ensureDriverPaths(): Promise<{ tauriDriver: string; nativeDriver?: string }> {
  const tauriDriver = process.env.SHELLPP_E2E_RESOLVED_TAURI_DRIVER ?? resolveTauriDriver();
  let nativeDriver = process.env.SHELLPP_E2E_RESOLVED_NATIVE_DRIVER;
  if (nativeDriver === undefined) {
    nativeDriver = (await resolveNativeDriver()) ?? '';
  }
  process.env.SHELLPP_E2E_RESOLVED_TAURI_DRIVER = tauriDriver;
  process.env.SHELLPP_E2E_RESOLVED_NATIVE_DRIVER = nativeDriver;
  return { tauriDriver, nativeDriver: nativeDriver === '' ? undefined : nativeDriver };
}

/**
 * Creates a fresh data directory and starts tauri-driver with
 * `SHELLPP_DATA_DIR` pointing to it; resolves when the driver accepts
 * connections.
 */
export async function startDriver(settings: DriverSettings, specs: string[], cid: string): Promise<string> {
  if (running) {
    await stopDriver();
  }
  const { tauriDriver, nativeDriver } = await ensureDriverPaths();
  await assertPortFree(settings.port);
  await assertPortFree(settings.nativePort);

  const dataDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'shellpp-e2e-'));
  const logs = logDir();
  await fsp.mkdir(logs, { recursive: true });
  const specName = specs[0] ? path.basename(specs[0].replace(/^file:\/\//, '')) : 'spec';
  const logFile = path.join(logs, `${safeName(`${cid}-${specName}`)}.tauri-driver.log`);
  const log = fs.createWriteStream(logFile, { flags: 'w' });

  const args = ['--port', String(settings.port), '--native-port', String(settings.nativePort)];
  if (nativeDriver) {
    args.push('--native-driver', nativeDriver);
  }
  const child = spawn(tauriDriver, args, {
    // The native driver and the app inherit this environment. With
    // SHELLPP_DATA_DIR set, the app also keeps its logs and temporary files
    // inside the directory (architecture 5.3).
    env: {
      ...process.env,
      SHELLPP_DATA_DIR: dataDir,
      RUST_BACKTRACE: process.env.RUST_BACKTRACE ?? '1',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  child.stdout?.pipe(log, { end: false });
  child.stderr?.pipe(log, { end: false });
  running = { process: child, dataDir, logFile, log };
  // Specs may read the directory (for example to check what was written);
  // helpers point to the log when the app goes away.
  process.env.SHELLPP_E2E_DATA_DIR = dataDir;
  process.env.SHELLPP_E2E_DRIVER_LOG = logFile;

  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `tauri-driver exited with code ${child.exitCode} before accepting connections.\n${tail(logFile)}`,
      );
    }
    if (await canConnect(settings.port)) {
      return dataDir;
    }
    await delay(100);
  }
  await stopDriver();
  throw new Error(`tauri-driver did not listen on port ${settings.port} within 20 s.\n${tail(logFile)}`);
}

function killTree(child: ChildProcess): void {
  if (child.exitCode !== null || child.pid === undefined) {
    return;
  }
  if (process.platform === 'win32') {
    // tauri-driver keeps msedgedriver (and through it the app) in a job
    // object, but kill the whole tree explicitly in case that changes.
    spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
  } else {
    // tauri-driver kills WebKitWebDriver on SIGTERM.
    child.kill('SIGTERM');
  }
}

async function waitForExit(child: ChildProcess, ms: number): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  await Promise.race([new Promise<void>((resolve) => child.once('exit', () => resolve())), delay(ms)]);
  if (child.exitCode === null && child.signalCode === null && process.platform !== 'win32') {
    child.kill('SIGKILL');
  }
}

/** Stops tauri-driver and deletes the data directory (`SHELLPP_E2E_KEEP_DATA=1` keeps it). */
export async function stopDriver(): Promise<void> {
  const current = running;
  running = undefined;
  if (!current) {
    return;
  }
  killTree(current.process);
  await waitForExit(current.process, 5_000);
  current.log.end();
  if (process.env.SHELLPP_E2E_KEEP_DATA === '1') {
    console.log(`[e2e] Kept data directory ${current.dataDir} (log: ${current.logFile})`);
    return;
  }
  try {
    // The app may release its files a moment after the process tree is gone.
    await fsp.rm(current.dataDir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  } catch (error) {
    console.warn(`[e2e] Could not delete ${current.dataDir}: ${(error as Error).message}`);
  }
}
