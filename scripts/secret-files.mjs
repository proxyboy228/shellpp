#!/usr/bin/env node
// Fails when any object of the repository's history, on any ref (branches,
// tags, stash), is named like a key store: gitleaks scans text and skips
// binary files, so a committed `.jks` or `.p12` would pass it (review of
// run 16; D224). Run from the working copy: `node scripts/secret-files.mjs`.

import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const KEY_STORE = /\.(jks|keystore|p12|pfx|bks|pkcs12)$/i;

const objects = execFileSync('git', ['rev-list', '--objects', '--all'], {
  cwd: repoRoot,
  encoding: 'utf8',
  maxBuffer: 512 * 1024 * 1024,
});
const found = new Set();
for (const line of objects.split('\n')) {
  const space = line.indexOf(' ');
  if (space < 0) {
    continue;
  }
  const path = line.slice(space + 1).trim();
  if (KEY_STORE.test(path)) {
    found.add(path);
  }
}
if (found.size > 0) {
  console.error('Key store files in the history (never commit them; D203, CONTRIBUTING):');
  for (const path of [...found].sort()) {
    console.error(`  ${path}`);
  }
  process.exitCode = 1;
} else {
  console.log('No key store files in the history.');
}
