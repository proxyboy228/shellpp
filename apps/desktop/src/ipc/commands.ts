// Typed command functions. Each rejects with an `IpcError`
// (docs/architecture.md, section 3.4; contract in ./types.ts).
//
// Owner: W4.

import { getBackend } from './backend';
import type * as T from './types';

function call<C extends T.CommandName>(command: C, input: T.CommandInput<C>): Promise<T.CommandOutput<C>> {
  return getBackend().invoke(command, input);
}

// app
export const appInfo = () => call('app_info', undefined);
export const stateSnapshot = () => call('state_snapshot', undefined);
export const windowOpen = () => call('window_open', undefined);
/** Keyboard or mouse input; call at most once every 30 seconds. */
export const activity = () => call('activity', undefined);
export const appExit = () => call('app_exit', undefined);
/** Opens an `https:` link in the system browser. */
export const externalOpenUrl = (url: string) => call('external_open_url', { url });

// lock
export const lockStatus = () => call('lock_status', undefined);
export const firstRunSetup = (password: string) => call('first_run_setup', { password });
export const unlock = (password: string) => call('unlock', { password });
export const lock = () => call('lock', undefined);
export const recoveryUnlock = (input: T.RecoveryUnlockInput) => call('recovery_unlock', input);
export const passwordChange = (input: T.PasswordChangeInput) => call('password_change', input);
export const recoveryKeyShow = (password: string) => call('recovery_key_show', { password });
export const recoveryKeyReissue = (password: string) => call('recovery_key_reissue', { password });
export const rememberDeviceSet = (enabled: boolean) => call('remember_device_set', { enabled });

// records
export const recordsQuery = (query: T.RecordsQuery) => call('records_query', query);
export const recordGet = (ref: T.RecordRef) => call('record_get', ref);
export const recordSave = (input: T.RecordSaveInput) => call('record_save', input);
export const recordDelete = (input: T.RecordsRef) => call('record_delete', input);
export const recordRestore = (input: T.RecordsRef) => call('record_restore', input);
export const recordMove = (input: T.RecordMoveInput) => call('record_move', input);
export const recordDuplicate = (ref: T.RecordRef) => call('record_duplicate', ref);
export const secretReveal = (field: T.SecretFieldRef) => call('secret_reveal', field);
export const secretCopy = (target: T.SecretCopyTarget) => call('secret_copy', target);

// settings
export const settingsGet = () => call('settings_get', undefined);
export const settingsSet = (input: T.SettingsSetInput) => call('settings_set', input);

// notifications
export const notificationsList = () => call('notifications_list', undefined);
export const notificationsMarkRead = () => call('notifications_mark_read', undefined);

// clipboard (non-sensitive text only; secrets go through `secretCopy`)
export const clipboardWriteText = (text: string) => call('clipboard_write_text', { text });
export const clipboardReadText = () => call('clipboard_read_text', undefined);

// e2e only: exists in builds with the `e2e-mocks` feature
export const e2eMock = (action: T.E2eMockAction) => call('e2e_mock', action);
