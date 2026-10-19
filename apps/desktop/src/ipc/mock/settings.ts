// Settings for the mock: every key of docs/data-model.md, section 16.1, with
// its default and validation (unknown key or bad value -> `validation_failed`
// with `field` = the key).
//
// Owner: W4.

import type { JsonValue, OsKind, ValidationReason } from '../types';

type Validator = (value: JsonValue) => ValidationReason | null;

interface SettingSpec {
  default: JsonValue;
  validate: Validator;
}

const isPlainObject = (value: JsonValue): value is { [key: string]: JsonValue } =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const oneOf =
  (...allowed: readonly JsonValue[]): Validator =>
  (value) =>
    allowed.includes(value) ? null : 'invalid';

const bool: Validator = (value) => (typeof value === 'boolean' ? null : 'invalid');

const intRange =
  (min: number, max: number): Validator =>
  (value) => {
    if (typeof value !== 'number' || !Number.isInteger(value)) {
      return 'invalid';
    }
    return value < min || value > max ? 'out_of_range' : null;
  };

const numberRange =
  (min: number, max: number): Validator =>
  (value) => {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
      return 'invalid';
    }
    return value < min || value > max ? 'out_of_range' : null;
  };

const stringOrNull: Validator = (value) => (value === null || typeof value === 'string' ? null : 'invalid');

const nonEmptyString: Validator = (value) =>
  typeof value === 'string' && value.trim().length > 0 && value.length <= 256 ? null : 'invalid';

const themeRef: Validator = (value) => {
  if (!isPlainObject(value)) {
    return 'invalid';
  }
  const keys = Object.keys(value);
  if (keys.length !== 1) {
    return 'invalid';
  }
  if (typeof value.builtin === 'string') {
    return value.builtin === 'graphite' || value.builtin === 'paper' ? null : 'invalid';
  }
  return typeof value.record_id === 'string' ? null : 'invalid';
};

const shortcutOverrides: Validator = (value) => {
  if (!isPlainObject(value)) {
    return 'invalid';
  }
  return Object.values(value).every((entry) => entry === null || typeof entry === 'string') ? null : 'invalid';
};

const stringList: Validator = (value) =>
  Array.isArray(value) && value.every((entry) => typeof entry === 'string') ? null : 'invalid';

/** `session.last` (docs/data-model.md 16.3): null or `{saved_at, tabs: [...]}` (shape checked by the UI on read). */
const lastSession: Validator = (value) =>
  value === null || (isPlainObject(value) && typeof value.saved_at === 'string' && Array.isArray(value.tabs)) ? null : 'invalid';

const SCREENS = ['hosts', 'keychain', 'forwarding', 'snippets', 'known_hosts'] as const;

/** The settings table for the given OS (`shortcuts.preset` depends on it). */
export function settingsSchema(os: OsKind): Readonly<Record<string, SettingSpec>> {
  const schema: Record<string, SettingSpec> = {
    'appearance.ui_theme': { default: 'auto', validate: oneOf('auto', 'light', 'dark') },
    'appearance.ui_zoom': { default: 1, validate: numberRange(0.5, 2) },
    'security.auto_lock_minutes': { default: 15, validate: oneOf(1, 5, 15, 30, 60, null) },
    'terminal.autocomplete': { default: 'off', validate: oneOf('off') },
    'terminal.autoreconnect': { default: true, validate: bool },
    'terminal.record_history': { default: true, validate: bool },
    'terminal.import_shell_history': { default: false, validate: bool },
    'terminal.copy_on_select': { default: false, validate: bool },
    'terminal.bell_sound': { default: false, validate: bool },
    'terminal.bold_is_bright': { default: true, validate: bool },
    'terminal.cursor_style': { default: 'block', validate: oneOf('block', 'bar', 'underline') },
    'terminal.cursor_blink': { default: false, validate: bool },
    'terminal.term': { default: 'xterm-256color', validate: oneOf('xterm', 'xterm-256color', 'vt100', 'linux') },
    'terminal.font_family': { default: 'JetBrains Mono', validate: nonEmptyString },
    'terminal.font_size': { default: 14, validate: intRange(6, 72) },
    'terminal.line_height': { default: 1.2, validate: numberRange(1, 2) },
    'terminal.confirm_multiline_paste': { default: true, validate: bool },
    'terminal.default_theme': { default: { builtin: 'graphite' }, validate: themeRef },
    'terminal.highlighting_enabled': { default: true, validate: bool },
    'terminal.os_detection_enabled': { default: true, validate: bool },
    'terminal.os_detection_script': { default: null, validate: stringOrNull },
    'terminal.local_shell': { default: null, validate: stringOrNull },
    'terminal.keepalive_s': { default: 30, validate: intRange(0, 3600) },
    'sftp.show_hidden': { default: false, validate: bool },
    'sftp.confirm_delete': { default: true, validate: bool },
    'sftp.name_conflict': { default: 'ask', validate: oneOf('ask', 'replace', 'skip', 'rename') },
    'sftp.parallel_transfers': { default: 3, validate: intRange(1, 10) },
    'sftp.open_program': { default: null, validate: stringOrNull },
    'logs.retention_days': { default: 90, validate: intRange(1, 3650) },
    'logs.record_output': { default: false, validate: bool },
    'shortcuts.preset': { default: os === 'macos' ? 'mac' : 'pc', validate: oneOf('pc', 'mac') },
    'shortcuts.overrides': { default: {}, validate: shortcutOverrides },
    'updates.auto_check': { default: false, validate: bool },
    'updates.manifest_url': { default: null, validate: stringOrNull },
    'view.details_panel_width': { default: 380, validate: intRange(320, 560) },
    'hints.dismissed': { default: [], validate: stringList },
    // Stage 2 (written by the terminal workspace, S6).
    'session.last': { default: null, validate: lastSession },
  };
  for (const screen of SCREENS) {
    schema[`view.${screen}.layout`] = { default: 'grid', validate: oneOf('grid', 'list') };
    schema[`view.${screen}.sort`] = {
      default: screen === 'hosts' ? 'manual' : 'name',
      validate: oneOf('manual', 'name', 'created', 'modified'),
    };
  }
  return schema;
}
