#!/usr/bin/env node
// Builds the desktop app for the UI e2e suite (stage plan, section 8.1):
// a debug build with the `e2e-mocks` feature, the e2e capability and the
// frontend built with `vite build --mode e2e`, without bundles.
//
//   pnpm --filter @shellpp/desktop e2e:build
//
// Output: <CARGO_TARGET_DIR or <repo>/target/e2e>/debug/shellpp(.exe).
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

const invokedDirectly = process.argv[1] !== undefined && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (invokedDirectly) {
  try {
    buildE2eApp(process.argv.slice(2).filter((arg) => arg !== '--'));
  } catch (error) {
    console.error(`[e2e-build] ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  }
}
