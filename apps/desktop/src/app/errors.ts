// User-facing error texts: command error codes map to `errors.<code>` and
// field reasons to `validation.<reason>` (docs/architecture.md, section 3.4).
//
// Owner: W4.

import { t } from '../i18n';
import { toIpcError, type ValidationReason } from '../ipc';
import { showToast } from './toast';

/** The message for any rejected command. */
export function errorMessage(reason: unknown): string {
  return t(`errors.${toIpcError(reason).code}`);
}

/** The generic message for a refused field. */
export function validationMessage(reason: ValidationReason): string {
  return t(`validation.${reason}`);
}

/** Shows a command error as a toast (`app-error-toast`). */
export function showErrorToast(reason: unknown): void {
  showToast({ message: errorMessage(reason), level: 'error', testId: 'app-error-toast' });
}
