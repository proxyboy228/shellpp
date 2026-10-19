// i18n catalog checks (plan, W4): no key in both files, every literal key used
// with t() exists, every error code and validation reason has a text, and
// placeholders are filled.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { ErrorCode, SshReason, ValidationReason } from '../ipc';
import { CATALOG_FILES, hasKey, t } from './index';

// Vitest runs in the package directory (apps/desktop).
const SRC = join(process.cwd(), 'src');

function sourceFiles(directory: string): string[] {
  const files: string[] = [];
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) {
      files.push(...sourceFiles(path));
    } else if (/\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name)) {
      files.push(path);
    }
  }
  return files;
}

const ERROR_CODES: readonly ErrorCode[] = [
  'locked',
  'wrong_state',
  'invalid_password',
  'invalid_recovery_key',
  'not_found',
  'validation_failed',
  'read_only',
  'too_large',
  'unavailable',
  'newer_schema',
  'damaged',
  'storage_error',
  'io_error',
  'internal',
  'cancelled',
  'ssh_error',
  'host_key_revoked',
];

const SSH_REASONS: readonly SshReason[] = [
  'not_found',
  'refused',
  'timeout',
  'network',
  'host_key_rejected',
  'auth_failed',
  'protocol',
  'channel_failed',
  'disconnected',
  'key_unusable',
];

const VALIDATION_REASONS: readonly ValidationReason[] = [
  'required',
  'too_long',
  'too_short',
  'invalid',
  'out_of_range',
  'duplicate',
  'too_many',
  'cycle',
  'too_deep',
  'not_allowed',
  'missing_ref',
];

describe('i18n catalog', () => {
  it('has no key in more than one file', () => {
    const seen = new Map<string, string>();
    const duplicates: string[] = [];
    for (const [file, catalog] of Object.entries(CATALOG_FILES)) {
      for (const key of Object.keys(catalog)) {
        const other = seen.get(key);
        if (other !== undefined) {
          duplicates.push(`${key} (${other}, ${file})`);
        }
        seen.set(key, file);
      }
    }
    expect(duplicates).toEqual([]);
  });

  it('uses flat dotted keys with non-empty English texts', () => {
    for (const catalog of Object.values(CATALOG_FILES)) {
      for (const [key, text] of Object.entries(catalog)) {
        expect(key).toMatch(/^[a-z0-9_]+(\.[a-z0-9_]+)+$|^[a-z0-9_]+\.[a-z0-9_]+$/);
        expect(typeof text).toBe('string');
        expect(text.trim()).not.toBe('');
      }
    }
  });

  it('contains every literal key passed to t()', () => {
    const missing: string[] = [];
    for (const file of sourceFiles(SRC)) {
      const source = readFileSync(file, 'utf8');
      for (const match of source.matchAll(/\bt\(\s*'([^'$`]+)'/g)) {
        if (!hasKey(match[1])) {
          missing.push(`${relative(SRC, file)}: ${match[1]}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it('has a text for every error code and validation reason', () => {
    for (const code of ERROR_CODES) {
      expect(hasKey(`errors.${code}`), code).toBe(true);
    }
    for (const reason of VALIDATION_REASONS) {
      expect(hasKey(`validation.${reason}`), reason).toBe(true);
    }
    for (const reason of SSH_REASONS) {
      expect(hasKey(`ssh_reason.${reason}`), reason).toBe(true);
    }
  });

  it('fills placeholders and falls back to the key', () => {
    expect(t('about.version', { version: '1.2.3' })).toBe('Version 1.2.3');
    expect(t('security.auto_lock.minutes', { count: 5 })).toBe('5 minutes');
    expect(t('about.version')).toBe('Version {version}');
    expect(t('no.such.key')).toBe('no.such.key');
  });

  it('has a text for the app menu, sidebar and settings labels built from keys', () => {
    const keys = [
      'menu.vaults',
      'menu.settings',
      'menu.edit',
      'menu.edit.undo',
      'menu.edit.redo',
      'menu.edit.cut',
      'menu.edit.copy',
      'menu.edit.paste',
      'menu.edit.select_all',
      'menu.view',
      'menu.view.zoom_in',
      'menu.view.zoom_out',
      'menu.view.zoom_reset',
      'menu.view.fullscreen',
      'menu.view.toggle_sidebar',
      'menu.window',
      'menu.window.new_window',
      'menu.help',
      'menu.help.documentation',
      'menu.help.report_issue',
      'menu.help.about',
      'menu.lock',
      'menu.exit',
      'sidebar.hosts',
      'settings.security',
      'settings.link.documentation',
      'settings.link.report_bug',
      'settings.link.changelog',
    ];
    for (const key of keys) {
      expect(hasKey(key), key).toBe(true);
    }
  });
});
