// Field validators of the host and group panels. They mirror the core's
// `record_save` rules (docs/data-model.md 1.1, 7.1-7.3; stage plan 5.5) so the
// panel can show the reason under a field before anything is sent. The core
// validates again; its `validation_failed` reasons are shown the same way.

import type { ValidationReason } from '../../ipc';

/** Result of parsing one form value: the normalized value (`undefined` = member absent) or a reason. */
export type ParseResult<T> = { ok: true; value: T | undefined } | { ok: false; reason: ValidationReason };

export const ok = <T>(value: T | undefined): ParseResult<T> => ({ ok: true, value });
export const fail = <T = never>(reason: ValidationReason): ParseResult<T> => ({ ok: false, reason });

/** True for a Unicode control character (general category Cc). */
function isControl(codePoint: number): boolean {
  return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f);
}

/** True when the text holds a control character other than the allowed ones. */
export function hasControl(text: string, allowed: readonly number[] = []): boolean {
  for (const char of text) {
    const codePoint = char.codePointAt(0) ?? 0;
    if (isControl(codePoint) && !allowed.includes(codePoint)) {
      return true;
    }
  }
  return false;
}

/** Number of Unicode scalar values. */
export function scalarLength(text: string): number {
  let count = 0;
  for (const char of text) {
    void char;
    count += 1;
  }
  return count;
}

const encoder = new TextEncoder();

/** Length in UTF-8 bytes. */
export function utf8Length(text: string): number {
  return encoder.encode(text).length;
}

/**
 * `string(n)`: NFC, trimmed, 1..n scalar values, no control characters. An
 * empty value means "absent" (`required` when the member is required).
 */
export function parseString(raw: string, max: number, required = false): ParseResult<string> {
  const value = raw.normalize('NFC').trim();
  if (value === '') {
    return required ? fail('required') : ok(undefined);
  }
  if (hasControl(value)) {
    return fail('invalid');
  }
  if (scalarLength(value) > max) {
    return fail('too_long');
  }
  return ok(value);
}

const LF = 0x0a;
const TAB = 0x09;

/** `text(n)`: multi-line, LF line endings, at most n UTF-8 bytes, no control characters except LF and TAB. */
export function parseText(raw: string, maxBytes: number): ParseResult<string> {
  const value = raw.replace(/\r\n?/g, '\n');
  if (value === '') {
    return ok(undefined);
  }
  if (hasControl(value, [LF, TAB])) {
    return fail('invalid');
  }
  if (utf8Length(value) > maxBytes) {
    return fail('too_long');
  }
  return ok(value);
}

/** `secret(n)`: kept exactly as typed, at most n UTF-8 bytes, no NUL. */
export function parseSecret(raw: string, maxBytes: number): ParseResult<string> {
  if (raw.includes('\u0000')) {
    return fail('invalid');
  }
  if (utf8Length(raw) > maxBytes) {
    return fail('too_long');
  }
  return ok(raw);
}

/** An integer in `min..max`; an empty field means "absent". */
export function parseInteger(raw: string, min: number, max: number): ParseResult<number> {
  const value = raw.trim();
  if (value === '') {
    return ok(undefined);
  }
  if (!/^\d+$/.test(value)) {
    return fail('invalid');
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < min || number > max) {
    return fail('out_of_range');
  }
  return ok(number);
}

export function isIpv4(text: string): boolean {
  const parts = text.split('.');
  return (
    parts.length === 4 &&
    parts.every((part) => /^(0|[1-9]\d{0,2})$/.test(part) && Number(part) <= 255)
  );
}

function isIpv6Address(text: string): boolean {
  const halves = text.split('::');
  if (halves.length > 2) {
    return false;
  }
  const split = (part: string): string[] => (part === '' ? [] : part.split(':'));
  const left = split(halves[0] ?? '');
  const right = halves.length === 2 ? split(halves[1] ?? '') : null;
  const tail = right ?? left;
  let groups = left.length + (right?.length ?? 0);
  const last = tail[tail.length - 1];
  if (last !== undefined && last.includes('.')) {
    if (!isIpv4(last)) {
      return false;
    }
    tail.pop();
    // The IPv4 tail stands for two groups.
    groups += 1;
  }
  if (![...left, ...(right ?? [])].every((group) => /^[0-9a-fA-F]{1,4}$/.test(group))) {
    return false;
  }
  return right === null ? groups === 8 : groups <= 7;
}

/** An IPv6 literal without brackets, with an optional zone index (`fe80::1%eth0`). */
export function isIpv6Literal(text: string): boolean {
  const percent = text.indexOf('%');
  if (percent < 0) {
    return isIpv6Address(text);
  }
  const zone = text.slice(percent + 1);
  return zone !== '' && !/[%:]/.test(zone) && isIpv6Address(text.slice(0, percent));
}

/**
 * Host address (data-model 7.2): required string(255); surrounding brackets
 * removed; no whitespace, `@`, `/`, `\`, `[`, `]`; a `:` only inside an IPv6
 * literal.
 */
export function parseAddress(raw: string): ParseResult<string> {
  let value = raw.normalize('NFC').trim();
  if (value === '') {
    return fail('required');
  }
  if (value.length >= 2 && value.startsWith('[') && value.endsWith(']')) {
    value = value.slice(1, -1).trim();
    if (value === '') {
      return fail('invalid');
    }
  }
  if (hasControl(value)) {
    return fail('invalid');
  }
  if (scalarLength(value) > 255) {
    return fail('too_long');
  }
  if (/[\s@/\\[\]]/u.test(value)) {
    return fail('invalid');
  }
  if (value.includes(':') && !isIpv6Literal(value)) {
    return fail('invalid');
  }
  return ok(value);
}

/** One tag: string(64) without commas. */
export function parseTag(raw: string): ParseResult<string> {
  if (raw.includes(',')) {
    return fail('invalid');
  }
  return parseString(raw, 64);
}

export const MAX_TAGS = 32;

/** Case-insensitive de-duplication; the first spelling is kept. */
export function dedupeTags(tags: readonly string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const tag of tags) {
    const key = tag.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      result.push(tag);
    }
  }
  return result;
}

/** A field error with its full dotted path (`ssh.env.2.name`). */
export interface PathError {
  path: string;
  reason: ValidationReason;
}

/** A list field: the normalized list or every error with its path. */
export type ListResult<T> = { ok: true; value: T[] | undefined } | { ok: false; errors: PathError[] };

/** Host tags: each a valid tag, at most 32 after de-duplication. */
export function parseTags(raw: readonly string[]): ListResult<string> {
  const errors: PathError[] = [];
  const values: string[] = [];
  raw.forEach((tag, index) => {
    const result = parseTag(tag);
    if (!result.ok) {
      errors.push({ path: `tags.${index}`, reason: result.reason });
    } else if (result.value !== undefined) {
      values.push(result.value);
    }
  });
  const unique = dedupeTags(values);
  if (unique.length > MAX_TAGS) {
    errors.push({ path: 'tags', reason: 'too_many' });
  }
  if (errors.length > 0) {
    return { ok: false, errors };
  }
  return { ok: true, value: unique.length === 0 ? undefined : unique };
}

export interface EnvRow {
  name: string;
  value: string;
}

export const MAX_ENV = 64;
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]{0,255}$/;

/**
 * `ssh.env`: at most 64 variables; names `[A-Za-z_][A-Za-z0-9_]{0,255}`,
 * unique (case-sensitive); values secret(1024). Rows with an empty name and
 * an empty value are being typed and are skipped.
 */
export function parseEnv(rows: readonly EnvRow[]): ListResult<EnvRow> {
  const errors: PathError[] = [];
  const values: EnvRow[] = [];
  const seen = new Set<string>();
  rows.forEach((row, index) => {
    const name = row.name.trim();
    if (name === '' && row.value === '') {
      return;
    }
    let rowOk = true;
    if (name === '') {
      errors.push({ path: `ssh.env.${index}.name`, reason: 'required' });
      rowOk = false;
    } else if (!ENV_NAME.test(name)) {
      errors.push({ path: `ssh.env.${index}.name`, reason: name.length > 256 ? 'too_long' : 'invalid' });
      rowOk = false;
    } else if (seen.has(name)) {
      errors.push({ path: `ssh.env.${index}.name`, reason: 'duplicate' });
      rowOk = false;
    }
    seen.add(name);
    const value = parseSecret(row.value, 1024);
    if (!value.ok) {
      errors.push({ path: `ssh.env.${index}.value`, reason: value.reason });
      rowOk = false;
    }
    if (rowOk) {
      values.push({ name, value: row.value });
    }
  });
  if (values.length > MAX_ENV) {
    errors.push({ path: 'ssh.env', reason: 'too_many' });
  }
  if (errors.length > 0) {
    return { ok: false, errors };
  }
  return { ok: true, value: values.length === 0 ? undefined : values };
}

/** Charsets offered by the panel (stage plan 6.6); `utf-8` is the default. */
export const CHARSETS = [
  'utf-8',
  'iso-8859-1',
  'iso-8859-2',
  'iso-8859-5',
  'iso-8859-15',
  'windows-1250',
  'windows-1251',
  'windows-1252',
  'koi8-r',
  'koi8-u',
  'gb18030',
  'gbk',
  'big5',
  'shift_jis',
  'euc-jp',
  'iso-2022-jp',
  'euc-kr',
] as const;

/**
 * `charset`: string(64), lowercase WHATWG encoding label matching
 * `[a-z0-9._:-]+` (stage plan 5.5; the class includes the underscore of
 * `shift_jis`), the same rule as the core.
 */
export function parseCharset(raw: string): ParseResult<string> {
  const value = raw.trim().toLowerCase();
  if (value === '') {
    return ok(undefined);
  }
  if (scalarLength(value) > 64) {
    return fail('too_long');
  }
  if (!/^[a-z0-9._:-]+$/.test(value)) {
    return fail('invalid');
  }
  return ok(value);
}
