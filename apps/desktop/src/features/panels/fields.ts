// Field tables of the host panel (UI 3.1) and the group panel (UI 3.4): how
// each form value maps to a data member and how it is validated.

import type { TagColor, ThemeRef } from '../../ipc';
import { TAG_COLORS } from '../../components';
import { fail, ok, parseAddress, parseCharset, parseEnv, parseInteger, parseString, parseTags, parseText } from './validators';
import type { EnvRow } from './validators';
import { list, single, type FieldDef, type FieldTable } from './form';

export type Backspace = 'default' | 'del' | 'ctrl_h';
export type RecordMode = 'default' | 'on' | 'off';
/** `null`: the global default theme (the member is removed). */
export type ThemeChoice = ThemeRef | null;

export const BACKSPACE_VALUES: readonly Backspace[] = ['default', 'del', 'ctrl_h'];
export const RECORD_MODES: readonly RecordMode[] = ['default', 'on', 'off'];

export interface HostForm {
  address: string;
  label: string;
  /** `''`: top level. */
  group_id: string;
  tags: string[];
  notes: string;
  color: TagColor;
  backspace: Backspace;
  port: string;
  username: string;
  /** Linked key (`ssh.key_id`, "+ -> Key"); `''`: none. */
  key_id: string;
  agent_forwarding: boolean;
  env: EnvRow[];
  charset: string;
  keepalive: string;
  record_history: RecordMode;
  record_output: RecordMode;
  theme: ThemeChoice;
}

export interface GroupForm {
  name: string;
  /** `''`: top level. */
  parent_id: string;
  color: TagColor;
  port: string;
  username: string;
}

const asString = (value: unknown): string => (typeof value === 'string' ? value : '');
const asNumberText = (value: unknown): string => (typeof value === 'number' ? String(value) : '');

function oneOf<T extends string>(values: readonly T[], fallback: T) {
  return (value: unknown): T => (values.includes(value as T) ? (value as T) : fallback);
}

function enumField<T extends string>(path: string, values: readonly T[], fallback: T): FieldDef<T> {
  return {
    path,
    defaultValue: fallback,
    toForm: oneOf(values, fallback),
    parse: (value) => single(path, values.includes(value) ? ok(value) : fail('invalid')),
  };
}

const colorField = (path: string): FieldDef<TagColor> => enumField(path, TAG_COLORS, 'graphite');

const uuidField = (path: string): FieldDef<string> => ({
  path,
  toForm: asString,
  parse: (value) => single(path, ok(value === '' ? undefined : value)),
});

function isThemeRef(value: unknown): value is ThemeRef {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const ref = value as Record<string, unknown>;
  return typeof ref.builtin === 'string' || typeof ref.record_id === 'string';
}

export const HOST_FIELDS: FieldTable<HostForm> = {
  address: {
    path: 'address',
    toForm: asString,
    parse: (value) => single('address', parseAddress(value)),
  },
  label: {
    path: 'label',
    toForm: asString,
    parse: (value) => single('label', parseString(value, 256)),
  },
  group_id: uuidField('group_id'),
  tags: {
    path: 'tags',
    defaultValue: [],
    toForm: (value) => (Array.isArray(value) ? value.filter((tag): tag is string => typeof tag === 'string') : []),
    parse: (value) => list(parseTags(value)),
  },
  notes: {
    path: 'notes',
    toForm: asString,
    parse: (value) => single('notes', parseText(value, 32768)),
  },
  color: colorField('color'),
  backspace: enumField('backspace', BACKSPACE_VALUES, 'default'),
  port: {
    path: 'ssh.port',
    toForm: asNumberText,
    parse: (value) => single('ssh.port', parseInteger(value, 1, 65535)),
  },
  username: {
    path: 'ssh.username',
    toForm: asString,
    parse: (value) => single('ssh.username', parseString(value, 256)),
  },
  key_id: uuidField('ssh.key_id'),
  agent_forwarding: {
    path: 'ssh.agent_forwarding',
    defaultValue: false,
    toForm: (value) => value === true,
    parse: (value) => ({ ok: true, value }),
  },
  env: {
    path: 'ssh.env',
    defaultValue: [],
    toForm: (value) =>
      Array.isArray(value)
        ? value.map((row: unknown) => {
            const entry = (typeof row === 'object' && row !== null ? row : {}) as Record<string, unknown>;
            return { name: asString(entry.name), value: asString(entry.value) };
          })
        : [],
    parse: (value) => list(parseEnv(value)),
  },
  charset: {
    path: 'charset',
    defaultValue: 'utf-8',
    toForm: (value) => (typeof value === 'string' && value !== '' ? value : 'utf-8'),
    parse: (value) => single('charset', parseCharset(value)),
  },
  keepalive: {
    path: 'ssh.keepalive_s',
    toForm: asNumberText,
    parse: (value) => single('ssh.keepalive_s', parseInteger(value, 0, 3600)),
  },
  record_history: enumField('record_history', RECORD_MODES, 'default'),
  record_output: enumField('record_output', RECORD_MODES, 'default'),
  theme: {
    path: 'terminal_theme',
    toForm: (value) => (isThemeRef(value) ? value : null),
    parse: (value) => ({ ok: true, value: value ?? undefined }),
  },
};

export const GROUP_FIELDS: FieldTable<GroupForm> = {
  name: {
    path: 'name',
    toForm: asString,
    parse: (value) => single('name', parseString(value, 256, true)),
  },
  parent_id: uuidField('parent_id'),
  color: colorField('color'),
  port: {
    path: 'ssh.port',
    toForm: asNumberText,
    parse: (value) => single('ssh.port', parseInteger(value, 1, 65535)),
  },
  username: {
    path: 'ssh.username',
    toForm: asString,
    parse: (value) => single('ssh.username', parseString(value, 256)),
  },
};
