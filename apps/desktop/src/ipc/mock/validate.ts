// `record_save` validation for the mock (plan, "record_save rules";
// docs/data-model.md, sections 1.1, 7.1-7.3). The mock reports the same field
// paths and reasons as the core; it shares nothing with the panel validators
// except the rules.
//
// Owner: W4.

import type { FieldError, TagColor, ValidationReason } from '../types';
import { isUuid } from './ids';

export type DataObject = Record<string, unknown>;

export const TAG_COLORS: readonly TagColor[] = ['graphite', 'jade', 'amber', 'coral', 'plum', 'moss', 'ocean', 'rose'];
export const HOST_SECRET_PATHS = ['ssh.password', 'ssh.proxy.password'] as const;

const encoder = new TextEncoder();
const byteLength = (text: string) => encoder.encode(text).length;
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
// eslint-disable-next-line no-control-regex
const TEXT_CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/;

export class ErrorList {
  readonly errors: FieldError[] = [];
  add(field: string, reason: ValidationReason): void {
    this.errors.push({ field, reason });
  }
  get empty(): boolean {
    return this.errors.length === 0;
  }
}

export function isObject(value: unknown): value is DataObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

type Normalized<T> = { ok: true; value: T | undefined } | { ok: false; reason: ValidationReason };

/** `string(n)`: NFC, trimmed, 1..n scalar values, no control characters; empty means absent. */
export function normalizeString(value: unknown, max: number): Normalized<string> {
  if (value === undefined || value === null) {
    return { ok: true, value: undefined };
  }
  if (typeof value !== 'string') {
    return { ok: false, reason: 'invalid' };
  }
  const text = value.normalize('NFC').trim();
  if (text === '') {
    return { ok: true, value: undefined };
  }
  if (CONTROL.test(text)) {
    return { ok: false, reason: 'invalid' };
  }
  if ([...text].length > max) {
    return { ok: false, reason: 'too_long' };
  }
  return { ok: true, value: text };
}

/** `text(n)`: at most n UTF-8 bytes, LF line endings, no control characters except LF and TAB. */
export function normalizeText(value: unknown, maxBytes: number): Normalized<string> {
  if (value === undefined || value === null) {
    return { ok: true, value: undefined };
  }
  if (typeof value !== 'string') {
    return { ok: false, reason: 'invalid' };
  }
  const text = value.replace(/\r\n?/g, '\n');
  if (text === '') {
    return { ok: true, value: undefined };
  }
  if (TEXT_CONTROL.test(text)) {
    return { ok: false, reason: 'invalid' };
  }
  if (byteLength(text) > maxBytes) {
    return { ok: false, reason: 'too_long' };
  }
  return { ok: true, value: text };
}

/** `secret(n)`: exactly as entered, at most n UTF-8 bytes, no NUL. */
export function checkSecret(value: string, maxBytes: number): ValidationReason | null {
  if (value.includes('\u0000')) {
    return 'invalid';
  }
  return byteLength(value) > maxBytes ? 'too_long' : null;
}

function isIpv6Literal(text: string): boolean {
  const [address, zone, extra] = text.split('%');
  if (address === undefined || extra !== undefined || (zone !== undefined && !/^[A-Za-z0-9_.-]+$/.test(zone))) {
    return false;
  }
  if (!address.includes(':')) {
    return false;
  }
  try {
    // The WHATWG URL parser validates IPv6 literals.
    new URL(`http://[${address}]/`);
    return true;
  } catch {
    return false;
  }
}

/** Host address rules (data-model 7.2). Returns the normalized address. */
export function normalizeAddress(value: unknown): Normalized<string> {
  const base = normalizeString(value, 255);
  if (!base.ok || base.value === undefined) {
    return base;
  }
  let text = base.value;
  if (text.startsWith('[') && text.endsWith(']')) {
    text = text.slice(1, -1);
  }
  if (text === '' || /[\s@/\\[\]]/.test(text)) {
    return { ok: false, reason: 'invalid' };
  }
  if (text.includes(':') && !isIpv6Literal(text)) {
    return { ok: false, reason: 'invalid' };
  }
  return { ok: true, value: text };
}

function checkInt(value: unknown, min: number, max: number): ValidationReason | null {
  if (typeof value !== 'number' || !Number.isInteger(value)) {
    return 'invalid';
  }
  return value < min || value > max ? 'out_of_range' : null;
}

/** Sets or removes `target[key]` from a normalization result; records the error. */
function apply(target: DataObject, key: string, field: string, result: Normalized<unknown>, errors: ErrorList): void {
  if (!result.ok) {
    errors.add(field, result.reason);
    return;
  }
  if (result.value === undefined) {
    delete target[key];
  } else {
    target[key] = result.value;
  }
}

function optionalUuid(target: DataObject, key: string, field: string, errors: ErrorList): void {
  const value = target[key];
  if (value === undefined || value === null || value === '') {
    delete target[key];
  } else if (!isUuid(value)) {
    errors.add(field, 'invalid');
  }
}

function optionalEnum(target: DataObject, key: string, field: string, allowed: readonly string[], errors: ErrorList): void {
  const value = target[key];
  if (value === undefined || value === null) {
    delete target[key];
  } else if (typeof value !== 'string' || !allowed.includes(value)) {
    errors.add(field, 'invalid');
  }
}

function optionalBool(target: DataObject, key: string, field: string, errors: ErrorList): void {
  const value = target[key];
  if (value === undefined || value === null) {
    delete target[key];
  } else if (typeof value !== 'boolean') {
    errors.add(field, 'invalid');
  }
}

function optionalInt(target: DataObject, key: string, field: string, min: number, max: number, errors: ErrorList): void {
  const value = target[key];
  if (value === undefined || value === null || value === '') {
    delete target[key];
    return;
  }
  const reason = checkInt(value, min, max);
  if (reason !== null) {
    errors.add(field, reason);
  }
}

function jumpChain(ssh: DataObject, prefix: string, selfId: string | null, errors: ErrorList): void {
  const value = ssh.jump_chain;
  if (value === undefined || value === null) {
    delete ssh.jump_chain;
    return;
  }
  if (!Array.isArray(value)) {
    errors.add(`${prefix}.jump_chain`, 'invalid');
    return;
  }
  if (value.length > 8) {
    errors.add(`${prefix}.jump_chain`, 'too_many');
    return;
  }
  const seen = new Set<string>();
  value.forEach((entry: unknown, index) => {
    if (!isUuid(entry) || entry === selfId) {
      errors.add(`${prefix}.jump_chain.${index}`, 'invalid');
    } else if (seen.has(entry)) {
      errors.add(`${prefix}.jump_chain.${index}`, 'duplicate');
    } else {
      seen.add(entry);
    }
  });
}

function sshBlock(data: DataObject, selfId: string | null, errors: ErrorList, kind: 'host' | 'group'): void {
  const value = data.ssh;
  if (value === undefined || value === null) {
    delete data.ssh;
    return;
  }
  if (!isObject(value)) {
    errors.add('ssh', 'invalid');
    return;
  }
  const ssh: DataObject = { ...value };
  data.ssh = ssh;
  if ('password' in ssh) {
    errors.add('ssh.password', 'not_allowed');
  }
  optionalInt(ssh, 'port', 'ssh.port', 1, 65535, errors);
  apply(ssh, 'username', 'ssh.username', normalizeString(ssh.username, 256), errors);
  optionalUuid(ssh, 'identity_id', 'ssh.identity_id', errors);
  optionalUuid(ssh, 'key_id', 'ssh.key_id', errors);
  jumpChain(ssh, 'ssh', selfId, errors);
  if (kind === 'group') {
    return;
  }
  optionalUuid(ssh, 'certificate_key_id', 'ssh.certificate_key_id', errors);
  optionalUuid(ssh, 'fido2_key_id', 'ssh.fido2_key_id', errors);
  optionalUuid(ssh, 'startup_snippet_id', 'ssh.startup_snippet_id', errors);
  optionalBool(ssh, 'agent_forwarding', 'ssh.agent_forwarding', errors);
  optionalBool(ssh, 'mosh', 'ssh.mosh', errors);
  optionalInt(ssh, 'keepalive_s', 'ssh.keepalive_s', 0, 3600, errors);
  proxy(ssh, errors);
  environment(ssh, errors);
}

function proxy(ssh: DataObject, errors: ErrorList): void {
  const value = ssh.proxy;
  if (value === undefined || value === null) {
    delete ssh.proxy;
    return;
  }
  if (!isObject(value)) {
    errors.add('ssh.proxy', 'invalid');
    return;
  }
  const next: DataObject = { ...value };
  ssh.proxy = next;
  if ('password' in next) {
    errors.add('ssh.proxy.password', 'not_allowed');
  }
  if (next.type !== 'http' && next.type !== 'socks5') {
    errors.add('ssh.proxy.type', 'invalid');
  }
  const address = normalizeAddress(next.address);
  if (address.ok && address.value === undefined) {
    errors.add('ssh.proxy.address', 'required');
  } else {
    apply(next, 'address', 'ssh.proxy.address', address, errors);
  }
  const portReason = checkInt(next.port, 1, 65535);
  if (portReason !== null) {
    errors.add('ssh.proxy.port', portReason);
  }
  apply(next, 'username', 'ssh.proxy.username', normalizeString(next.username, 256), errors);
}

function environment(ssh: DataObject, errors: ErrorList): void {
  const value = ssh.env;
  if (value === undefined || value === null) {
    delete ssh.env;
    return;
  }
  if (!Array.isArray(value)) {
    errors.add('ssh.env', 'invalid');
    return;
  }
  if (value.length > 64) {
    errors.add('ssh.env', 'too_many');
    return;
  }
  const names = new Set<string>();
  ssh.env = value.map((entry: unknown, index) => {
    if (!isObject(entry)) {
      errors.add(`ssh.env.${index}`, 'invalid');
      return entry;
    }
    const name = entry.name;
    if (typeof name !== 'string' || !/^[A-Za-z_][A-Za-z0-9_]{0,255}$/.test(name)) {
      errors.add(`ssh.env.${index}.name`, 'invalid');
    } else if (names.has(name)) {
      errors.add(`ssh.env.${index}.name`, 'duplicate');
    } else {
      names.add(name);
    }
    const envValue = entry.value ?? '';
    if (typeof envValue !== 'string') {
      errors.add(`ssh.env.${index}.value`, 'invalid');
    } else {
      const reason = checkSecret(envValue, 1024);
      if (reason !== null) {
        errors.add(`ssh.env.${index}.value`, reason);
      }
    }
    return { ...entry, value: envValue };
  });
}

function tags(data: DataObject, errors: ErrorList): void {
  const value = data.tags;
  if (value === undefined || value === null) {
    delete data.tags;
    return;
  }
  if (!Array.isArray(value)) {
    errors.add('tags', 'invalid');
    return;
  }
  const result: string[] = [];
  const seen = new Set<string>();
  value.forEach((entry: unknown, index) => {
    const normalized = normalizeString(entry, 64);
    if (!normalized.ok) {
      errors.add(`tags.${index}`, normalized.reason);
      return;
    }
    if (normalized.value === undefined) {
      return;
    }
    if (normalized.value.includes(',')) {
      errors.add(`tags.${index}`, 'invalid');
      return;
    }
    const key = normalized.value.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      result.push(normalized.value);
    }
  });
  if (result.length > 32) {
    errors.add('tags', 'too_many');
    return;
  }
  data.tags = result;
}

function color(data: DataObject, errors: ErrorList): void {
  optionalEnum(data, 'color', 'color', TAG_COLORS, errors);
}

function terminalTheme(data: DataObject, errors: ErrorList): void {
  const value = data.terminal_theme;
  if (value === undefined || value === null) {
    delete data.terminal_theme;
    return;
  }
  if (!isObject(value) || Object.keys(value).length !== 1) {
    errors.add('terminal_theme', 'invalid');
    return;
  }
  if ('builtin' in value) {
    if (value.builtin !== 'graphite' && value.builtin !== 'paper') {
      errors.add('terminal_theme', 'invalid');
    }
  } else if (!isUuid(value.record_id)) {
    errors.add('terminal_theme', 'invalid');
  }
}

function charset(data: DataObject, errors: ErrorList): void {
  const value = data.charset;
  if (value === undefined || value === null || value === '') {
    delete data.charset;
    return;
  }
  const normalized = normalizeString(value, 64);
  if (!normalized.ok) {
    errors.add('charset', normalized.reason);
    return;
  }
  if (normalized.value === undefined) {
    delete data.charset;
    return;
  }
  const lower = normalized.value.toLowerCase();
  if (!/^[a-z0-9._:-]+$/.test(lower)) {
    errors.add('charset', 'invalid');
    return;
  }
  data.charset = lower;
}

/**
 * Validates and normalizes a host `data` object (members the mock does not
 * know are kept). Managed members are handled by the caller.
 */
export function validateHostData(input: DataObject, selfId: string | null): { data: DataObject; errors: FieldError[] } {
  const errors = new ErrorList();
  const data: DataObject = { ...input };
  const address = normalizeAddress(data.address);
  if (address.ok && address.value === undefined) {
    errors.add('address', 'required');
  } else {
    apply(data, 'address', 'address', address, errors);
  }
  apply(data, 'label', 'label', normalizeString(data.label, 256), errors);
  optionalUuid(data, 'group_id', 'group_id', errors);
  tags(data, errors);
  apply(data, 'notes', 'notes', normalizeText(data.notes, 32768), errors);
  color(data, errors);
  optionalEnum(data, 'backspace', 'backspace', ['default', 'del', 'ctrl_h'], errors);
  charset(data, errors);
  terminalTheme(data, errors);
  optionalEnum(data, 'record_history', 'record_history', ['default', 'on', 'off'], errors);
  optionalEnum(data, 'record_output', 'record_output', ['default', 'on', 'off'], errors);
  if (data.telnet !== undefined && data.telnet !== null) {
    if (!isObject(data.telnet)) {
      errors.add('telnet', 'invalid');
    } else {
      const telnet: DataObject = { ...data.telnet };
      optionalInt(telnet, 'port', 'telnet.port', 1, 65535, errors);
      data.telnet = telnet;
    }
  } else {
    delete data.telnet;
  }
  const protocol = data.primary_protocol;
  if (protocol === undefined || protocol === null) {
    delete data.primary_protocol;
  } else if (protocol !== 'ssh' && !(protocol === 'telnet' && data.telnet !== undefined)) {
    errors.add('primary_protocol', 'invalid');
  }
  sshBlock(data, selfId, errors, 'host');
  return { data, errors: errors.errors };
}

/** Validates and normalizes a group `data` object (tree rules are checked by the caller). */
export function validateGroupData(input: DataObject): { data: DataObject; errors: FieldError[] } {
  const errors = new ErrorList();
  const data: DataObject = { ...input };
  const name = normalizeString(data.name, 256);
  if (name.ok && name.value === undefined) {
    errors.add('name', 'required');
  } else {
    apply(data, 'name', 'name', name, errors);
  }
  optionalUuid(data, 'parent_id', 'parent_id', errors);
  color(data, errors);
  apply(data, 'notes', 'notes', normalizeText(data.notes, 32768), errors);
  sshBlock(data, null, errors, 'group');
  return { data, errors: errors.errors };
}
