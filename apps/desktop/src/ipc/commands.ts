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
/** Opens a new window; with an intent it opens a session or a local terminal first (UI 2.4 Alt, 4.4). */
export const windowOpen = (intent?: T.WindowIntent) => call('window_open', intent === undefined ? undefined : { intent });
/** Keyboard or mouse input; call at most once every 30 seconds. */
export const activity = () => call('activity', undefined);
/** Exits unless active sessions need confirmation (then `needs_confirmation`; call again with `confirmed`). */
export const appExit = (confirmed = false) => call('app_exit', { confirmed });
/** Closes this window unless its active sessions need confirmation. */
export const windowClose = (confirmed = false) => call('window_close', { confirmed });
/** Opens an `https:` link in the system browser (`http:` too for terminal links). */
export const externalOpenUrl = (url: string, source?: 'terminal') =>
  call('external_open_url', source === undefined ? { url } : { url, source });

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

// sessions (stage 2)
export const sessionOpen = (input: T.SessionOpenInput) => call('session_open', input);
/** Attaches the output channel: buffered output first, then live output, in order. */
export const sessionAttach = (input: T.SessionAttachInput) => call('session_attach', input);
export const sessionWrite = (input: T.SessionWriteInput) => call('session_write', input);
export const sessionInject = (input: T.SessionInjectInput) => call('session_inject', input);
export const sessionResize = (input: T.SessionResizeInput) => call('session_resize', input);
export const sessionPromptReply = (input: T.SessionPromptReplyInput) => call('session_prompt_reply', input);
/** Cancels a connection attempt or stops automatic reconnect attempts. */
export const sessionCancel = (sessionId: T.Uuid) => call('session_cancel', { session_id: sessionId });
/** Ends the session and forgets it (final `closed`). */
export const sessionClose = (sessionId: T.Uuid) => call('session_close', { session_id: sessionId });
/** Reconnect / Retry in the same tab. */
export const sessionReconnect = (sessionId: T.Uuid) => call('session_reconnect', { session_id: sessionId });
/** Flow control: the terminal processed `bytes` more bytes of output. */
export const sessionAck = (input: T.SessionAckInput) => call('session_ack', input);

// local terminal
export const localShells = () => call('local_shells', undefined);
export const localOpen = (input: T.LocalOpenInput) => call('local_open', input);

// known hosts
export const knownHostsList = () => call('known_hosts_list', undefined);
export const knownHostRemove = (input: T.RecordsRef) => call('known_host_remove', input);

// keys ("+ -> Key")
export const keysList = () => call('keys_list', undefined);
export const keyImport = (input: T.KeyImportInput) => call('key_import', input);

// history and recent connections
export const historyList = (input: T.HistoryListInput) => call('history_list', input);
export const historyDelete = (input: T.RecordsRef) => call('history_delete', input);
export const historyClear = (input: T.HistoryClearInput) => call('history_clear', input);
export const recentConnections = () => call('recent_connections', undefined);

// e2e only: exists in builds with the `e2e-mocks` feature
export const e2eMock = (action: T.E2eMockAction) => call('e2e_mock', action);
