#!/usr/bin/env node
// Fails when a production npm dependency of the workspace has a license
// outside the allowlist (see THIRD_PARTY_NOTICES.md and docs/decisions.md,
// decision 4). Run from anywhere: `node scripts/check-licenses.mjs`.
//
// License expressions use SPDX syntax. "A OR B" passes when any alternative
// is allowed, "A AND B" only when every part is allowed, and "A WITH E"
// (a license exception, which only grants extra permissions) when "A" is.

import { execSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ALLOWED = new Set([
  'MIT',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'ISC',
  'MPL-2.0',
  '0BSD',
  'Unlicense',
  'CC0-1.0',
  'Zlib',
  'BSL-1.0',
  'Unicode-3.0',
  'CDLA-Permissive-2.0',
]);

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** Splits an SPDX expression into parentheses, operators and license ids. */
function tokenize(expression) {
  const tokens = expression.match(/\(|\)|[^\s()]+/g) ?? [];
  return tokens.map((token) => {
    const upper = token.toUpperCase();
    return upper === 'OR' || upper === 'AND' || upper === 'WITH' ? upper : token;
  });
}

/**
 * Evaluates an SPDX license expression against the allowlist.
 * Grammar: or := and ("OR" and)*; and := with ("AND" with)*;
 * with := primary ("WITH" exception)?; primary := id | "(" or ")".
 * Returns false for anything it cannot parse.
 */
function isAllowedExpression(expression) {
  const tokens = tokenize(expression);
  let position = 0;

  const peek = () => tokens[position];
  const next = () => tokens[position++];

  function parseOr() {
    let result = parseAnd();
    while (peek() === 'OR') {
      next();
      const right = parseAnd();
      result = result || right;
    }
    return result;
  }

  function parseAnd() {
    let result = parseWith();
    while (peek() === 'AND') {
      next();
      const right = parseWith();
      result = result && right;
    }
    return result;
  }

  function parseWith() {
    const result = parsePrimary();
    if (peek() === 'WITH') {
      next();
      const exception = next();
      if (exception === undefined || exception === '(' || exception === ')') {
        throw new Error('expected a license exception after WITH');
      }
    }
    return result;
  }

  function parsePrimary() {
    const token = next();
    if (token === '(') {
      const result = parseOr();
      if (next() !== ')') {
        throw new Error('missing closing parenthesis');
      }
      return result;
    }
    if (token === undefined || token === ')' || token === 'OR' || token === 'AND' || token === 'WITH') {
      throw new Error(`unexpected token ${token ?? 'end of expression'}`);
    }
    // "Apache-2.0+" means "this version or later"; the named version decides.
    return ALLOWED.has(token.endsWith('+') ? token.slice(0, -1) : token);
  }

  try {
    const result = parseOr();
    return position === tokens.length && result;
  } catch {
    return false;
  }
}

function listProductionLicenses() {
  const output = execSync('pnpm licenses list --prod --json', {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'inherit'],
  }).trim();

  if (output === '' || /^no licenses/i.test(output)) {
    return {};
  }
  return JSON.parse(output);
}

function main() {
  const byLicense = listProductionLicenses();
  const violations = [];
  let packageCount = 0;

  for (const [groupLicense, packages] of Object.entries(byLicense)) {
    for (const pkg of packages) {
      packageCount += 1;
      const license = typeof pkg.license === 'string' && pkg.license.trim() !== '' ? pkg.license : groupLicense;
      if (!isAllowedExpression(license)) {
        violations.push(`${pkg.name}@${(pkg.versions ?? []).join(', ')}: ${license}`);
      }
    }
  }

  if (violations.length > 0) {
    console.error(`License check failed: ${violations.length} production package(s) outside the allowlist.`);
    for (const violation of violations) {
      console.error(`  - ${violation}`);
    }
    console.error(`Allowed: ${[...ALLOWED].join(', ')}.`);
    process.exitCode = 1;
    return;
  }

  console.log(`License check passed: ${packageCount} production package(s), all licenses allowed.`);
}

main();
