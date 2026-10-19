// Paths used by the e2e harness. Kept in sync with scripts/e2e-build.mjs,
// which builds the binaries that `applicationPath()` and `sshdPath()` point to.

import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** `apps/desktop/e2e`. */
export const E2E_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
/** `apps/desktop`. */
export const DESKTOP_DIR = path.resolve(E2E_DIR, '..');
/** Repository root. */
export const REPO_ROOT = path.resolve(DESKTOP_DIR, '..', '..');

const EXE = process.platform === 'win32' ? '.exe' : '';

/** Cargo target directory of the e2e build: `CARGO_TARGET_DIR`, else `<repo>/target/e2e`. */
export function e2eTargetDir(): string {
  const fromEnv = process.env.CARGO_TARGET_DIR;
  return fromEnv ? path.resolve(fromEnv) : path.join(REPO_ROOT, 'target', 'e2e');
}

/** The debug binary built with the `e2e-mocks` feature (`SHELLPP_E2E_APP` overrides it). */
export function applicationPath(): string {
  const fromEnv = process.env.SHELLPP_E2E_APP;
  return fromEnv ? path.resolve(fromEnv) : path.join(e2eTargetDir(), 'debug', `shellpp${EXE}`);
}

/**
 * The test SSH server `shellpp-test-sshd` built by scripts/e2e-build.mjs into
 * the same target directory (`SHELLPP_E2E_SSHD` overrides it).
 */
export function sshdPath(): string {
  const fromEnv = process.env.SHELLPP_E2E_SSHD;
  return fromEnv ? path.resolve(fromEnv) : path.join(e2eTargetDir(), 'debug', `shellpp-test-sshd${EXE}`);
}

/** Directory for driver logs and WebdriverIO worker logs (gitignored). */
export function logDir(): string {
  return process.env.SHELLPP_E2E_LOG_DIR
    ? path.resolve(process.env.SHELLPP_E2E_LOG_DIR)
    : path.join(REPO_ROOT, 'target', 'e2e-logs');
}

/** Cache directory for downloaded native drivers (gitignored). */
export function driverCacheDir(): string {
  return process.env.SHELLPP_E2E_DRIVER_CACHE
    ? path.resolve(process.env.SHELLPP_E2E_DRIVER_CACHE)
    : path.join(REPO_ROOT, 'target', 'e2e-drivers');
}
