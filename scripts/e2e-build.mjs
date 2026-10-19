#!/usr/bin/env node
// Builds the binaries of the UI e2e suite (stage 1 plan, section 8.1; stage 2
// plan, section 8.1):
//
// 1. the desktop app: a debug build with the `e2e-mocks` feature, the e2e
//    capability and the frontend built with `vite build --mode e2e`, without
//    bundles;
// 2. the russh test SSH server `shellpp-test-sshd` (debug), which the specs
//    start through apps/desktop/e2e/support/sshd.ts.
//
//   pnpm --filter @shellpp/desktop e2e:build [-- --sshd-only | --app-only] [tauri build args]
//
// Output, both in <CARGO_TARGET_DIR or <repo>/target/e2e>/debug/:
//   shellpp(.exe)                (SHELLPP_E2E_APP overrides the path for the runner)
//   shellpp-test-sshd(.exe)      (SHELLPP_E2E_SSHD overrides the path for the runner)
// The Tauri CLI enables `tauri/custom-protocol` for `tauri build`, so the
// binary serves the bundled frontend (never the Vite dev server).
//
// The configuration override lives in apps/desktop/e2e/tauri.e2e.conf.json
// (passed by path, which avoids shell quoting of inline JSON on Windows):
// beforeBuildCommand `pnpm build:e2e` and capabilities ["default", "e2e"].
// Extra arguments go to `tauri build` after it, for example a further
// `--config <file>` (merged last) or `--verbose`.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const DESKTOP_DIR = path.join(REPO_ROOT, 'apps', 'desktop');
const EXE = process.platform === 'win32' ? '.exe' : '';

/** Cargo target directory of the e2e build. */
export function e2eTargetDir() {
  return process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(REPO_ROOT, 'target', 'e2e');
}

/** Path of the e2e binary (`SHELLPP_E2E_APP` overrides it). */
export function e2eAppPath() {
  return process.env.SHELLPP_E2E_APP
    ? path.resolve(process.env.SHELLPP_E2E_APP)
    : path.join(e2eTargetDir(), 'debug', `shellpp${EXE}`);
}

/** Path of the test SSH server binary (`SHELLPP_E2E_SSHD` overrides it). */
export function e2eSshdPath() {
  return process.env.SHELLPP_E2E_SSHD
    ? path.resolve(process.env.SHELLPP_E2E_SSHD)
    : path.join(e2eTargetDir(), 'debug', `shellpp-test-sshd${EXE}`);
}

/** Runs the Tauri CLI build; throws when it fails. Returns the binary path. */
export function buildE2eApp(extraArgs = []) {
  const targetDir = e2eTargetDir();
  const tauriCli = path.join(DESKTOP_DIR, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
  if (!fs.existsSync(tauriCli)) {
    throw new Error(`Tauri CLI not found at ${tauriCli}: run "pnpm install" first.`);
  }
  const configOverride = path.join(DESKTOP_DIR, 'e2e', 'tauri.e2e.conf.json');
  const args = [
    tauriCli,
    'build',
    '--debug',
    '--no-bundle',
    '--features',
    'e2e-mocks',
    '--config',
    configOverride,
    ...extraArgs,
  ];
  console.log(`[e2e-build] CARGO_TARGET_DIR=${targetDir}`);
  console.log(`[e2e-build] tauri ${args.slice(1).join(' ')}`);
  const result = spawnSync(process.execPath, args, {
    cwd: DESKTOP_DIR,
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
    stdio: 'inherit',
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`tauri build failed with exit code ${result.status}`);
  }
  const binary = path.join(targetDir, 'debug', `shellpp${EXE}`);
  if (!fs.existsSync(binary)) {
    throw new Error(`tauri build finished, but ${binary} does not exist`);
  }
  console.log(`[e2e-build] built ${binary}`);
  return binary;
}

/**
 * Builds the test SSH server (`cargo build -p shellpp-test-sshd --locked`,
 * debug) into the same target directory; throws when it fails. Returns the
 * binary path. `CARGO` selects the cargo binary (default: PATH).
 */
export function buildE2eSshd() {
  const targetDir = e2eTargetDir();
  const cargo = process.env.CARGO ?? 'cargo';
  const args = ['build', '-p', 'shellpp-test-sshd', '--bin', 'shellpp-test-sshd', '--locked'];
  console.log(`[e2e-build] CARGO_TARGET_DIR=${targetDir}`);
  console.log(`[e2e-build] cargo ${args.join(' ')}`);
  const result = spawnSync(cargo, args, {
    cwd: REPO_ROOT,
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
    stdio: 'inherit',
    windowsHide: true,
  });
  if (result.error) {
    throw new Error(`cannot run ${cargo}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`cargo build -p shellpp-test-sshd failed with exit code ${result.status}`);
  }
  const binary = path.join(targetDir, 'debug', `shellpp-test-sshd${EXE}`);
  if (!fs.existsSync(binary)) {
    throw new Error(`cargo build finished, but ${binary} does not exist`);
  }
  console.log(`[e2e-build] built ${binary}`);
  return binary;
}

/** Builds what the flags ask for: both by default, `--sshd-only` or `--app-only`. */
export function buildE2e(argv = []) {
  const args = argv.filter((arg) => arg !== '--');
  const sshdOnly = args.includes('--sshd-only');
  const appOnly = args.includes('--app-only');
  if (sshdOnly && appOnly) {
    throw new Error('--sshd-only and --app-only exclude each other');
  }
  const tauriArgs = args.filter((arg) => arg !== '--sshd-only' && arg !== '--app-only');
  // The test server first: it builds in seconds and fails fast.
  if (!appOnly) {
    buildE2eSshd();
  }
  if (!sshdOnly) {
    buildE2eApp(tauriArgs);
  }
}

const invokedDirectly = process.argv[1] !== undefined && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (invokedDirectly) {
  try {
    buildE2e(process.argv.slice(2));
  } catch (error) {
    console.error(`[e2e-build] ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  }
}
