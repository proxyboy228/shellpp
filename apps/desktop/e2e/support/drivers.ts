// Finds the WebDriver binaries the e2e suite needs (docs/architecture.md,
// section 9.1):
//
// - tauri-driver (`cargo install tauri-driver --locked`): `TAURI_DRIVER`, else
//   PATH, else `$CARGO_HOME/bin`.
// - the native driver tauri-driver forwards to:
//   - Windows: msedgedriver with the same version as the installed WebView2
//     runtime. `SHELLPP_E2E_NATIVE_DRIVER` wins; otherwise the cached driver
//     in `target/e2e-drivers/msedgedriver-<version>/`, a matching
//     msedgedriver on PATH, or a download through the `edgedriver` package
//     (MIT, a dependency of WebdriverIO; its install script is disabled in
//     pnpm-workspace.yaml, so the download happens here, once per version).
//   - Linux: WebKitWebDriver (package `webkit2gtk-driver` on Debian/Ubuntu,
//     `webkit2gtk4.1-devel` on Fedora): `SHELLPP_E2E_NATIVE_DRIVER`, else
//     PATH (tauri-driver also searches PATH itself).
//   - macOS has no WebDriver for WKWebView: the suite does not run there.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { driverCacheDir, REPO_ROOT } from './paths.ts';

const IS_WINDOWS = process.platform === 'win32';
const EXE = IS_WINDOWS ? '.exe' : '';

/** WebView2 Evergreen runtime client id (EdgeUpdate registry key). */
const WEBVIEW2_CLIENT_ID = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

function isFile(candidate: string): boolean {
  try {
    return fs.statSync(candidate).isFile();
  } catch {
    return false;
  }
}

/** Searches PATH for an executable. */
export function which(name: string): string | undefined {
  const dirs = (process.env.PATH ?? '').split(path.delimiter).filter(Boolean);
  const names = IS_WINDOWS && !name.toLowerCase().endsWith('.exe') ? [`${name}.exe`, name] : [name];
  for (const dir of dirs) {
    for (const candidate of names) {
      const full = path.join(dir, candidate);
      if (isFile(full)) {
        return full;
      }
    }
  }
  return undefined;
}

/** Path of tauri-driver, or an error with installation instructions. */
export function resolveTauriDriver(): string {
  const fromEnv = process.env.TAURI_DRIVER;
  if (fromEnv) {
    if (!isFile(fromEnv)) {
      throw new Error(`TAURI_DRIVER points to a missing file: ${fromEnv}`);
    }
    return fromEnv;
  }
  const onPath = which('tauri-driver');
  if (onPath) {
    return onPath;
  }
  const cargoHome = process.env.CARGO_HOME ?? path.join(os.homedir(), '.cargo');
  const inCargo = path.join(cargoHome, 'bin', `tauri-driver${EXE}`);
  if (isFile(inCargo)) {
    return inCargo;
  }
  throw new Error(
    'tauri-driver was not found. Install it with "cargo install tauri-driver --locked" ' +
      'or set TAURI_DRIVER to its path.',
  );
}

/** Version of the installed WebView2 runtime (Windows), for example `155.0.4283.62`. */
export function detectWebView2Version(): string {
  const fromEnv = process.env.SHELLPP_E2E_WEBVIEW2_VERSION;
  if (fromEnv) {
    return fromEnv;
  }
  const keys = [
    `HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT_ID}`,
    `HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT_ID}`,
    `HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2_CLIENT_ID}`,
  ];
  for (const key of keys) {
    try {
      const output = execFileSync('reg', ['query', key, '/v', 'pv'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
        windowsHide: true,
      });
      const match = /\bpv\s+REG_SZ\s+(\d+\.\d+\.\d+\.\d+)/.exec(output);
      if (match && match[1] !== '0.0.0.0') {
        return match[1];
      }
    } catch {
      // Key absent: try the next location.
    }
  }
  // Fallback: the versioned folders of a per-machine installation.
  const programFiles = process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)';
  const appDir = path.join(programFiles, 'Microsoft', 'EdgeWebView', 'Application');
  try {
    const versions = fs
      .readdirSync(appDir)
      .filter((entry) => /^\d+\.\d+\.\d+\.\d+$/.test(entry))
      .sort(compareVersions);
    const newest = versions.at(-1);
    if (newest) {
      return newest;
    }
  } catch {
    // Not installed per machine.
  }
  throw new Error(
    'Could not detect the WebView2 runtime version. Install the WebView2 runtime or set ' +
      'SHELLPP_E2E_WEBVIEW2_VERSION (or SHELLPP_E2E_NATIVE_DRIVER to a matching msedgedriver).',
  );
}

function compareVersions(a: string, b: string): number {
  const left = a.split('.').map(Number);
  const right = b.split('.').map(Number);
  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const diff = (left[index] ?? 0) - (right[index] ?? 0);
    if (diff !== 0) {
      return diff;
    }
  }
  return 0;
}

/** `msedgedriver --version` -> `155.0.4283.62`, or undefined. */
function msedgedriverVersion(binary: string): string | undefined {
  try {
    const output = execFileSync(binary, ['--version'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
      windowsHide: true,
      timeout: 15_000,
    });
    return /(\d+\.\d+\.\d+\.\d+)/.exec(output)?.[1];
  } catch {
    return undefined;
  }
}

function major(version: string): string {
  return version.split('.')[0] ?? version;
}

interface EdgedriverModule {
  download(edgeVersion?: string, cacheDir?: string): Promise<string>;
}

/**
 * Loads the `edgedriver` package. It is a dependency of WebdriverIO, not of
 * this package, so it is looked up through pnpm's virtual store when a plain
 * import fails.
 */
async function loadEdgedriver(): Promise<EdgedriverModule> {
  const specifier = 'edgedriver';
  try {
    return (await import(specifier)) as EdgedriverModule;
  } catch {
    const hoisted = path.join(REPO_ROOT, 'node_modules', '.pnpm', 'node_modules', 'edgedriver');
    const manifest = path.join(hoisted, 'package.json');
    if (!isFile(manifest)) {
      throw new Error(
        'The "edgedriver" package (installed with WebdriverIO) was not found. Run "pnpm install", ' +
          'or set SHELLPP_E2E_NATIVE_DRIVER to an msedgedriver matching the WebView2 runtime.',
      );
    }
    const pkg = JSON.parse(fs.readFileSync(manifest, 'utf8')) as {
      exports?: { '.'?: { import?: { default?: string } } };
    };
    const entry = pkg.exports?.['.']?.import?.default ?? './dist/index.js';
    return (await import(pathToFileURL(path.join(hoisted, entry)).href)) as EdgedriverModule;
  }
}

/** msedgedriver matching the installed WebView2 runtime (Windows). */
async function resolveMsedgedriver(): Promise<string> {
  const version = detectWebView2Version();
  const cacheDir = path.join(driverCacheDir(), `msedgedriver-${version}`);
  const cached = path.join(cacheDir, 'msedgedriver.exe');
  if (isFile(cached)) {
    return cached;
  }
  const onPath = which('msedgedriver');
  if (onPath) {
    const pathVersion = msedgedriverVersion(onPath);
    if (pathVersion && major(pathVersion) === major(version)) {
      return onPath;
    }
    console.warn(
      `[e2e] Ignoring ${onPath} (version ${pathVersion ?? 'unknown'}): WebView2 is ${version}.`,
    );
  }
  console.log(`[e2e] Downloading msedgedriver ${version} for WebView2 ${version} into ${cacheDir}`);
  const edgedriver = await loadEdgedriver();
  fs.mkdirSync(cacheDir, { recursive: true });
  const downloaded = await edgedriver.download(version, cacheDir);
  const downloadedVersion = msedgedriverVersion(downloaded);
  if (downloadedVersion && major(downloadedVersion) !== major(version)) {
    throw new Error(
      `Downloaded msedgedriver ${downloadedVersion} does not match WebView2 ${version}.`,
    );
  }
  return downloaded;
}

/**
 * Path of the native WebDriver for tauri-driver's `--native-driver`, or
 * undefined to let tauri-driver search PATH (Linux without an override).
 */
export async function resolveNativeDriver(): Promise<string | undefined> {
  const fromEnv = process.env.SHELLPP_E2E_NATIVE_DRIVER;
  if (fromEnv) {
    if (!isFile(fromEnv)) {
      throw new Error(`SHELLPP_E2E_NATIVE_DRIVER points to a missing file: ${fromEnv}`);
    }
    return fromEnv;
  }
  if (IS_WINDOWS) {
    return resolveMsedgedriver();
  }
  if (process.platform === 'linux') {
    const found = which('WebKitWebDriver');
    if (!found) {
      throw new Error(
        'WebKitWebDriver was not found on PATH. Install "webkit2gtk-driver" (Debian/Ubuntu) ' +
          'or set SHELLPP_E2E_NATIVE_DRIVER.',
      );
    }
    return found;
  }
  throw new Error(`UI e2e tests do not run on ${process.platform} (no WebDriver for the system webview).`);
}
