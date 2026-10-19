#!/usr/bin/env node
// npm license check (docs/architecture.md, sections 8.1 and 8.3; decision 4
// in docs/decisions.md). Run from anywhere: `node scripts/check-licenses.mjs`
// or `pnpm licenses:check`.
//
// - Production packages (what `pnpm licenses list --prod` reports) must use a
//   license of the shipped list and must be recorded in THIRD_PARTY_NOTICES.md
//   with their exact version.
// - Development packages (everything else that is installed) must use a
//   license of the development list: the shipped list plus BlueOak-1.0.0 and
//   CC-BY-4.0.
// - LICENSE_OVERRIDES replaces missing license metadata that was verified by
//   hand. PENDING_OWNER_DECISION lists development-only packages whose license
//   is outside the development list and waits for an owner decision; they are
//   printed as warnings. Anything else fails the check.
//
// License expressions use SPDX syntax. "A OR B" passes when any alternative
// is allowed, "A AND B" only when every part is allowed, and "A WITH E"
// (a license exception, which only grants extra permissions) when "A" is.

import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Shipped list (architecture 8.1): Rust crates, npm `dependencies`. */
const SHIPPED = new Set([
  'MIT',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'ISC',
  'MPL-2.0',
  'Zlib',
  'BSL-1.0',
  'Unicode-3.0',
  'CC0-1.0',
  'Unlicense',
  '0BSD',
  'CDLA-Permissive-2.0',
]);

/** Development list (architecture 8.1): never shipped build, lint and test tools. */
const DEVELOPMENT = new Set([...SHIPPED, 'BlueOak-1.0.0', 'CC-BY-4.0']);

/**
 * Packages whose license metadata is missing or wrong, verified by hand.
 * Key: `name@version`, so that a new version is checked again.
 */
const LICENSE_OVERRIDES = new Map([
  [
    'css-value@0.0.1',
    {
      license: 'MIT',
      reason: 'package.json has no "license" field; its Readme.md carries the MIT License text (checked by hand)',
    },
  ],
]);

/**
 * Development-only packages with a license outside the development list
 * (stage 1 plan, decision 21). Each entry matches a package name (string or
 * pattern) together with its exact declared license, so a license change is
 * caught. Never applies to production packages.
 */
const PENDING_OWNER_DECISION = [
  {
    name: 'argparse',
    license: 'PSF-2.0',
    reason: 'dev only, via WebdriverIO -> mocha -> js-yaml; Python Software Foundation License 2.0',
  },
  {
    name: 'spdx-exceptions',
    license: 'CC-BY-3.0',
    reason: 'dev only, via create-wdio; a data file (the SPDX license exception list)',
  },
  {
    name: /^@csstools\//,
    license: 'MIT-0',
    reason: 'dev only, via jsdom CSS parsing (stage 0); MIT without the attribution clause',
  },
];

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const NOTICES_FILE = 'THIRD_PARTY_NOTICES.md';

/** Splits an SPDX expression into parentheses, operators and license ids. */
function tokenize(expression) {
  const tokens = expression.match(/\(|\)|[^\s()]+/g) ?? [];
  return tokens.map((token) => {
    const upper = token.toUpperCase();
    return upper === 'OR' || upper === 'AND' || upper === 'WITH' ? upper : token;
  });
}

/**
 * Evaluates an SPDX license expression against an allowlist.
 * Grammar: or := and ("OR" and)*; and := with ("AND" with)*;
 * with := primary ("WITH" exception)?; primary := id | "(" or ")".
 * Returns false for anything it cannot parse.
 */
function isAllowedExpression(expression, allowed) {
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
    return allowed.has(token.endsWith('+') ? token.slice(0, -1) : token);
  }

  try {
    const result = parseOr();
    return position === tokens.length && result;
  } catch {
    return false;
  }
}

/** Runs `pnpm licenses list` and returns `name@version` -> package info. */
function listPackages(prodOnly) {
  const command = `pnpm licenses list${prodOnly ? ' --prod' : ''} --json`;
  const output = execSync(command, {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'inherit'],
  }).trim();

  const packages = new Map();
  if (output === '' || /^no licenses/i.test(output)) {
    return packages;
  }
  for (const [groupLicense, entries] of Object.entries(JSON.parse(output))) {
    for (const entry of entries) {
      const declared = typeof entry.license === 'string' && entry.license.trim() !== '' ? entry.license : groupLicense;
      for (const version of entry.versions ?? []) {
        packages.set(`${entry.name}@${version}`, { name: entry.name, version, license: declared.trim() });
      }
    }
  }
  return packages;
}

/** Reads `package -> Set(version)` from the npm table of THIRD_PARTY_NOTICES.md. */
function readNoticesNpmTable() {
  const text = readFileSync(resolve(repoRoot, NOTICES_FILE), 'utf8');
  const section = text.split(/^## /m).find((part) => part.startsWith('npm packages'));
  const recorded = new Map();
  if (section === undefined) {
    return recorded;
  }
  for (const line of section.split('\n')) {
    const cells = line.split('|').map((cell) => cell.trim());
    // A table row is "| name | version | license | source |".
    if (cells.length < 4 || cells[1] === '' || cells[1] === 'Package' || /^-+$/.test(cells[1])) {
      continue;
    }
    const versions = recorded.get(cells[1]) ?? new Set();
    for (const version of cells[2].split(',')) {
      versions.add(version.trim());
    }
    recorded.set(cells[1], versions);
  }
  return recorded;
}

function findPendingDecision(pkg) {
  return PENDING_OWNER_DECISION.find(
    (entry) =>
      entry.license === pkg.license &&
      (typeof entry.name === 'string' ? entry.name === pkg.name : entry.name.test(pkg.name)),
  );
}

function main() {
  const production = listPackages(true);
  const all = listPackages(false);
  const development = new Map([...all].filter(([key]) => !production.has(key)));

  const violations = [];
  const warnings = [];
  const usedOverrides = new Set();
  const usedPending = new Set();

  const effectiveLicense = (key, pkg) => {
    const override = LICENSE_OVERRIDES.get(key);
    if (override === undefined) {
      return pkg.license;
    }
    usedOverrides.add(key);
    return override.license;
  };

  for (const [key, pkg] of production) {
    const license = effectiveLicense(key, pkg);
    if (!isAllowedExpression(license, SHIPPED)) {
      violations.push(`production ${key}: ${license} is outside the shipped list`);
    }
  }

  for (const [key, pkg] of development) {
    const license = effectiveLicense(key, pkg);
    if (isAllowedExpression(license, DEVELOPMENT)) {
      continue;
    }
    const pending = findPendingDecision({ ...pkg, license });
    if (pending === undefined) {
      violations.push(`development ${key}: ${license} is outside the development list`);
    } else {
      usedPending.add(pending);
      warnings.push(`development ${key}: ${license}, pending owner decision (${pending.reason})`);
    }
  }

  const recorded = readNoticesNpmTable();
  for (const [key, pkg] of production) {
    if (!(recorded.get(pkg.name)?.has(pkg.version) ?? false)) {
      violations.push(`production ${key} is not recorded with this version in ${NOTICES_FILE} (npm packages)`);
    }
  }

  for (const key of LICENSE_OVERRIDES.keys()) {
    if (!usedOverrides.has(key)) {
      console.log(`Note: license override for ${key} is no longer used and can be removed.`);
    }
  }
  for (const entry of PENDING_OWNER_DECISION) {
    if (!usedPending.has(entry)) {
      console.log(`Note: pending exception ${String(entry.name)} (${entry.license}) is no longer used and can be removed.`);
    }
  }

  for (const warning of warnings) {
    console.warn(`Warning: ${warning}`);
  }

  if (violations.length > 0) {
    console.error(`License check failed: ${violations.length} problem(s).`);
    for (const violation of violations) {
      console.error(`  - ${violation}`);
    }
    console.error(`Shipped list: ${[...SHIPPED].join(', ')}.`);
    console.error(`Development list: the shipped list plus ${[...DEVELOPMENT].filter((id) => !SHIPPED.has(id)).join(', ')}.`);
    process.exitCode = 1;
    return;
  }

  console.log(
    `License check passed: ${production.size} production package(s) on the shipped list and recorded in ` +
      `${NOTICES_FILE}; ${development.size} development package(s) on the development list` +
      (warnings.length > 0 ? `, ${warnings.length} pending owner decision.` : '.'),
  );
}

main();
