// Command errors: every command rejects with `{code, details}`
// (docs/architecture.md, section 3.4). The UI maps `code` to `errors.<code>`.
//
// Owner: W4.

import type { CommandErrorPayload, ErrorCode, FieldError } from './types';

// A record keyed by `ErrorCode`, so a code added to the type without being
// listed here fails the type check.
const CODES: Record<ErrorCode, true> = {
  locked: true,
  wrong_state: true,
  invalid_password: true,
  invalid_recovery_key: true,
  not_found: true,
  validation_failed: true,
  read_only: true,
  too_large: true,
  unavailable: true,
  newer_schema: true,
  damaged: true,
  storage_error: true,
  io_error: true,
  internal: true,
  // stage 2
  cancelled: true,
  ssh_error: true,
  host_key_revoked: true,
};

const KNOWN_CODES: ReadonlySet<string> = new Set(Object.keys(CODES));

/** A rejected command. */
export class IpcError extends Error {
  readonly code: ErrorCode;
  readonly details: CommandErrorPayload['details'];

  constructor(code: ErrorCode, details: CommandErrorPayload['details'] = {}) {
    super(code);
    this.name = 'IpcError';
    this.code = code;
    this.details = details;
  }

  /** Field errors of `validation_failed`, else an empty list. */
  get fieldErrors(): readonly FieldError[] {
    return this.details.errors ?? [];
  }

  /** The reason for one field path, if it was refused. */
  fieldReason(field: string): FieldError['reason'] | undefined {
    return this.fieldErrors.find((error) => error.field === field)?.reason;
  }
}

function isPayload(value: unknown): value is CommandErrorPayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    typeof (value as { code: unknown }).code === 'string'
  );
}

/** Normalizes anything a command rejected with into an `IpcError`. */
export function toIpcError(reason: unknown): IpcError {
  if (reason instanceof IpcError) {
    return reason;
  }
  if (isPayload(reason) && KNOWN_CODES.has(reason.code)) {
    const details = typeof reason.details === 'object' && reason.details !== null ? reason.details : {};
    return new IpcError(reason.code, details);
  }
  return new IpcError('internal');
}

/** True when `reason` is an `IpcError`, optionally with the given code. */
export function isIpcError(reason: unknown, code?: ErrorCode): reason is IpcError {
  return reason instanceof IpcError && (code === undefined || reason.code === code);
}
