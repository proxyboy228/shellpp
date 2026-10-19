// WebdriverIO configuration for the UI e2e suite (docs/architecture.md,
// section 9; stage plan, section 8.1).
//
// Run it through `pnpm --filter @shellpp/desktop e2e` (scripts/e2e-run.mjs),
// which builds or reuses the debug binary with the `e2e-mocks` feature and
// starts WebdriverIO with Node >= 22.19. Every spec file runs in its own
// worker: `beforeSession` creates a fresh temporary SHELLPP_DATA_DIR and starts
// tauri-driver with it, `afterSession` stops the driver and deletes the
// directory. A spec restarts the app with `browser.reloadSession()` (same
// driver, same data directory).

import fs from 'node:fs';
import path from 'node:path';
import { applicationPath, E2E_DIR, logDir } from './support/paths.ts';
import { ensureDriverPaths, startDriver, stopDriver } from './support/session.ts';

const DRIVER_PORT = Number(process.env.SHELLPP_E2E_DRIVER_PORT ?? 4444);
const NATIVE_PORT = Number(process.env.SHELLPP_E2E_NATIVE_PORT ?? 4445);
const APPLICATION = applicationPath();

export const config: WebdriverIO.Config = {
  runner: 'local',
  tsConfigPath: path.join(E2E_DIR, 'tsconfig.json'),

  // Spec files are named after the UI section they cover (architecture 9.4).
  // The harness self-check (smoke/) runs only on request:
  // `--spec e2e/smoke/harness.e2e.ts`.
  specs: ['./specs/**/*.e2e.ts'],
  // One app at a time (architecture 3.1); the driver ports are fixed.
  maxInstances: 1,

  capabilities: [
    {
      // tauri-driver speaks WebDriver classic only.
      'wdio:enforceWebDriverClassic': true,
      'tauri:options': { application: APPLICATION },
    },
  ],

  // A driver is already running (started in beforeSession): WebdriverIO
  // connects to it instead of starting a browser driver of its own.
  hostname: '127.0.0.1',
  port: DRIVER_PORT,

  logLevel: 'warn',
  outputDir: path.join(logDir(), 'wdio'),
  bail: 0,
  waitforTimeout: 10_000,
  waitforInterval: 100,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 1,
  // Linux CI wraps the run in `xvfb-run` (e2e/README.md), so WebdriverIO must
  // not start a display server of its own.
  displayServerEnabled: false,

  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    // Argon2id runs at full strength (KDF parameters are never weakened in
    // tests) and restarts relaunch the app.
    timeout: 300_000,
  },

  onPrepare: async () => {
    if (!fs.existsSync(APPLICATION)) {
      throw new Error(
        `The e2e app binary is missing: ${APPLICATION}\n` +
          'Build it with "pnpm --filter @shellpp/desktop e2e:build" or set SHELLPP_E2E_APP.',
      );
    }
    // Resolve (and, on Windows, download once) the drivers in the launcher;
    // the workers inherit the paths through the environment.
    const drivers = await ensureDriverPaths();
    console.log(`[e2e] app: ${APPLICATION}`);
    console.log(`[e2e] tauri-driver: ${drivers.tauriDriver}`);
    console.log(`[e2e] native driver: ${drivers.nativeDriver ?? '(from PATH)'}`);
  },

  beforeSession: async (_config, _capabilities, specs, cid) => {
    await startDriver({ port: DRIVER_PORT, nativePort: NATIVE_PORT }, specs, cid);
  },

  afterSession: async () => {
    await stopDriver();
  },
};
