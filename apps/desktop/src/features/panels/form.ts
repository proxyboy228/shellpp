// A small form engine for the record panels (UI 1.3 saving rule).
//
// The form holds what the user typed; `base` is the record data from the
// core. A field whose value is invalid keeps the base value ("the last valid
// value stays"), so `buildData` always produces valid data. Members the panel
// does not edit are carried over from `base` unchanged (round-trip rule).

import type { FieldError, ValidationReason } from '../../ipc';
import type { ListResult, ParseResult, PathError } from './validators';

export type FieldParse = { ok: true; value: unknown } | { ok: false; errors: PathError[] };

export interface FieldDef<V> {
  /** Dotted data path of the member (`ssh.port`); also the error path prefix. */
  path: string;
  /** Value of an absent member: an absent member and its default compare equal. */
  defaultValue?: unknown;
  /** Form value for a data value (`undefined` when the member is absent). */
  toForm(value: unknown): V;
  /** Normalized data value (`undefined` removes the member) or the errors. */
  parse(value: V): FieldParse;
}

export type FieldTable<F> = { readonly [K in keyof F]: FieldDef<F[K]> };

export type Data = Readonly<Record<string, unknown>>;

/** Adapts a single-value parser result to a field result with the field path. */
export function single<T>(path: string, result: ParseResult<T>): FieldParse {
  return result.ok ? { ok: true, value: result.value } : { ok: false, errors: [{ path, reason: result.reason }] };
}

/** Adapts a list parser result. */
export function list<T>(result: ListResult<T>): FieldParse {
  return result.ok ? { ok: true, value: result.value } : { ok: false, errors: result.errors };
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Deep JSON equality; `undefined` and `null` members count as absent. */
export function jsonEqual(a: unknown, b: unknown): boolean {
  if (a === b) {
    return true;
  }
  if ((a === undefined || a === null) && (b === undefined || b === null)) {
    return true;
  }
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((item, index) => jsonEqual(item, b[index]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
    for (const key of keys) {
      if (!jsonEqual(a[key], b[key])) {
        return false;
      }
    }
    return true;
  }
  return false;
}

export function getPath(data: Data | undefined, path: string): unknown {
  let current: unknown = data;
  for (const segment of path.split('.')) {
    if (!isPlainObject(current)) {
      return undefined;
    }
    current = current[segment];
  }
  return current;
}

/** A copy of `data` with the member at `path` set (`undefined` removes it). */
export function setPath(data: Data, path: string, value: unknown): Record<string, unknown> {
  const [head, ...rest] = path.split('.');
  const copy: Record<string, unknown> = { ...data };
  if (head === undefined) {
    return copy;
  }
  if (rest.length === 0) {
    if (value === undefined) {
      delete copy[head];
    } else {
      copy[head] = value;
    }
    return copy;
  }
  const child = isPlainObject(copy[head]) ? (copy[head] as Data) : {};
  copy[head] = setPath(child, rest.join('.'), value);
  return copy;
}

function effective(def: FieldDef<unknown>, value: unknown): unknown {
  return value === undefined || value === null ? def.defaultValue : value;
}

function sameValue(def: FieldDef<unknown>, a: unknown, b: unknown): boolean {
  return jsonEqual(effective(def, a), effective(def, b));
}

function entries<F>(table: FieldTable<F>): [keyof F, FieldDef<unknown>][] {
  return Object.entries(table) as [keyof F, FieldDef<unknown>][];
}

/** The form for a record's data. */
export function formFromData<F>(table: FieldTable<F>, data: Data): F {
  const form = {} as F;
  for (const [key, def] of entries(table)) {
    form[key] = def.toForm(getPath(data, def.path)) as F[keyof F];
  }
  return form;
}

export interface Validation<F> {
  /** Normalized values of the valid fields. */
  values: Partial<Record<keyof F, unknown>>;
  /** Every error with its full path. */
  errors: PathError[];
  /** Keys of invalid fields. */
  invalid: ReadonlySet<keyof F>;
}

export function validateForm<F>(table: FieldTable<F>, form: F): Validation<F> {
  const values: Partial<Record<keyof F, unknown>> = {};
  const errors: PathError[] = [];
  const invalid = new Set<keyof F>();
  for (const [key, def] of entries(table)) {
    const result = def.parse(form[key]);
    if (result.ok) {
      values[key] = result.value;
    } else {
      errors.push(...result.errors);
      invalid.add(key);
    }
  }
  return { values, errors, invalid };
}

/**
 * Data to save: `base` with every valid field that differs applied; invalid
 * and skipped fields keep the base value. `changed` is false when the result
 * equals `base`.
 */
export function buildData<F>(
  table: FieldTable<F>,
  base: Data,
  validation: Validation<F>,
  skip: ReadonlySet<keyof F> = new Set(),
): { data: Record<string, unknown>; changed: boolean } {
  let data: Record<string, unknown> = { ...base };
  let changed = false;
  for (const [key, def] of entries(table)) {
    if (validation.invalid.has(key) || skip.has(key) || !(key in validation.values)) {
      continue;
    }
    const value = validation.values[key];
    if (!sameValue(def, value, getPath(base, def.path))) {
      data = setPath(data, def.path, value);
      changed = true;
    }
  }
  return { data, changed };
}

/**
 * Carries a new base into the form: a field that was in sync with the old
 * base takes the new value; a field the user changed keeps the typed value.
 */
export function syncForm<F>(table: FieldTable<F>, form: F, oldBase: Data, newBase: Data): F {
  let next = form;
  for (const [key, def] of entries(table)) {
    const before = getPath(oldBase, def.path);
    const after = getPath(newBase, def.path);
    if (sameValue(def, before, after)) {
      continue;
    }
    const current = def.parse(form[key]);
    if (current.ok && sameValue(def, current.value, before)) {
      if (next === form) {
        next = { ...form };
      }
      next[key] = def.toForm(after) as F[keyof F];
    }
  }
  return next;
}

/** The form key whose path is the field path or its prefix. */
export function keyForField<F>(table: FieldTable<F>, field: string): keyof F | undefined {
  for (const [key, def] of entries(table)) {
    if (field === def.path || field.startsWith(`${def.path}.`)) {
      return key;
    }
  }
  return undefined;
}

/** A refusal by the core, kept while the field still holds the refused value. */
export interface ServerError<F> {
  key: keyof F;
  raw: unknown;
  error: PathError;
}

export function toServerErrors<F>(table: FieldTable<F>, form: F, errors: readonly FieldError[]): ServerError<F>[] {
  const result: ServerError<F>[] = [];
  for (const error of errors) {
    const key = keyForField(table, error.field);
    if (key !== undefined) {
      result.push({ key, raw: form[key], error: { path: error.field, reason: error.reason } });
    }
  }
  return result;
}

/** Server errors whose field still holds the refused value. */
export function activeServerErrors<F>(errors: readonly ServerError<F>[], form: F): ServerError<F>[] {
  return errors.filter((entry) => jsonEqual(form[entry.key], entry.raw));
}

/** The reason for an exact path, if any. */
export function reasonFor(errors: readonly PathError[], path: string): ValidationReason | undefined {
  return errors.find((error) => error.path === path)?.reason;
}
