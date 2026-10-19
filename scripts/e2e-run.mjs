#!/usr/bin/env node
// Runs the UI e2e suite (stage plan, section 8.1):
//
//   pnpm --filter @shellpp/desktop e2e [-- <wdio options>]
//
// 1. Finds the e2e binaries (scripts/e2e-build.mjs): the app and the test
//    SSH server `shellpp-test-sshd`; builds the missing ones, or both with
//    --rebuild (--no-build fails instead).
// 2. Starts WebdriverIO with Node >= 22.19 (WebdriverIO 10 and edgedriver
//    require it). When this script runs on an older Node, it looks for a
//    newer one: SHELLPP_E2E_NODE, then nvm (nvm-windows: NVM_HOME,
//    %LOCALAPPDATA%\nvm, %APPDATA%\nvm; nvm: ~/.nvm/versions/node).
//
// Every other argument goes to `wdio run`, for example
//   pnpm --filter @shellpp/desktop e2e -- --spec e2e/specs/13.1-first-run.e2e.ts
// (paths relative to apps/desktop).

import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { buildE2eApp, buildE2eSshd, DESKTOP_DIR, e2eAppPath, e2eSshdPath } from './e2e-build.mjs';

const MIN_NODE = [22, 19, 0];

function parseVersion(text) {
  const match = /v?(\d+)\.(\d+)\.(\d+)/.exec(text);
  return match ? match.slice(1, 4).map(Number) : undefined;
}

function compare(a, b) {
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) {
      return a[index] - b[index];
    }
  }
  return 0;
}

function nodeVersionOf(binary) {
  try {
    return parseVersion(execFileSync(binary, ['--version'], { encoding: 'utf8', windowsHide: true }));
  } catch {
    return undefined;
  }
}

function nvmCandidates() {
  const candidates = [];
  const windowsRoots = [
    process.env.NVM_HOME,
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'nvm'),
    process.env.APPDATA && path.join(process.env.APPDATA, 'nvm'),
  ].filter(Boolean);
  const unixRoots = [process.env.NVM_DIR && path.join(process.env.NVM_DIR, 'versions', 'node'), path.join(os.homedir(), '.nvm', 'versions', 'node')].filter(Boolean);
  for (const root of process.platform === 'win32' ? windowsRoots : unixRoots) {
    let entries = [];
    try {
      entries = fs.readdirSync(root);
    } catch {
      continue;
    }
    for (const entry of entries) {
      const version = parseVersion(entry);
      if (!version) {
        continue;
      }
      const binary = process.platform === 'win32' ? path.join(root, entry, 'node.exe') : path.join(root, entry, 'bin', 'node');
      if (fs.existsSync(binary)) {
        candidates.push({ binary, version });
      }
    }
  }
  return candidates;
}

/** A Node binary >= 22.19, preferring the current one. */
function findNode() {
  const current = parseVersion(process.version);
  if (current && compare(current, MIN_NODE) >= 0) {
    return process.execPath;
  }
  const fromEnv = process.env.SHELLPP_E2E_NODE;
  if (fromEnv) {
    const version = nodeVersionOf(fromEnv);
    if (!version || compare(version, MIN_NODE) < 0) {
      throw new Error(`SHELLPP_E2E_NODE (${fromEnv}) is not Node >= ${MIN_NODE.join('.')}.`);
    }
    return fromEnv;
  }
  const best = nvmCandidates()
    .filter((candidate) => compare(candidate.version, MIN_NODE) >= 0)
    .sort((a, b) => compare(b.version, a.version))[0];
  if (best) {
    return best.binary;
  }
  throw new Error(
    `WebdriverIO needs Node >= ${MIN_NODE.join('.')} (this is ${process.version}). ` +
      'Install it (for example with nvm, without switching the default) and set SHELLPP_E2E_NODE to its node binary.',
  );
}

function main() {
  const args = process.argv.slice(2).filter((arg) => arg !== '--');
  const rebuild = args.includes('--rebuild');
  const noBuild = args.includes('--no-build');
  const wdioArgs = args.filter((arg) => arg !== '--rebuild' && arg !== '--no-build');

  // The test SSH server first: it builds in seconds and fails fast.
  const sshd = e2eSshdPath();
  if (rebuild || !fs.existsSync(sshd)) {
    if (noBuild) {
      throw new Error(`The test SSH server binary is missing: ${sshd} (run "pnpm --filter @shellpp/desktop e2e:build").`);
    }
    if (process.env.SHELLPP_E2E_SSHD && !rebuild) {
      throw new Error(`SHELLPP_E2E_SSHD points to a missing file: ${sshd}`);
    }
    buildE2eSshd();
  } else {
    console.log(`[e2e] reusing ${sshd} (pass --rebuild to rebuild)`);
  }

  const app = e2eAppPath();
  if (rebuild || !fs.existsSync(app)) {
    if (noBuild) {
      throw new Error(`The e2e binary is missing: ${app} (run "pnpm --filter @shellpp/desktop e2e:build").`);
    }
    if (process.env.SHELLPP_E2E_APP && !rebuild) {
      throw new Error(`SHELLPP_E2E_APP points to a missing file: ${app}`);
    }
    buildE2eApp();
  } else {
    console.log(`[e2e] reusing ${app} (pass --rebuild to rebuild)`);
  }

  const node = findNode();
  const wdio = path.join(DESKTOP_DIR, 'node_modules', '@wdio', 'cli', 'bin', 'wdio.js');
  if (!fs.existsSync(wdio)) {
    throw new Error(`WebdriverIO CLI not found at ${wdio}: run "pnpm install" first.`);
  }
  const config = path.join(DESKTOP_DIR, 'e2e', 'wdio.conf.ts');
  if (node !== process.execPath) {
    console.log(`[e2e] running WebdriverIO with ${node}`);
  }
  const result = spawnSync(node, [wdio, 'run', config, ...wdioArgs], {
    cwd: DESKTOP_DIR,
    env: { ...process.env, SHELLPP_E2E_APP: app, SHELLPP_E2E_SSHD: sshd },
    stdio: 'inherit',
  });
  if (result.error) {
    throw result.error;
  }
  process.exit(result.status ?? 1);
}

try {
  main();
} catch (error) {
  console.error(`[e2e] ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}
