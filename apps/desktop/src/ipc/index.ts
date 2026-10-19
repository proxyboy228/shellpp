// The only entry point to the Rust core for the UI (docs/architecture.md,
// section 2.5). Import from `src/ipc`, never from its files or from
// `@tauri-apps/api`.
//
// Owner: W4.

export * from './commands';
export { IpcError, isIpcError, toIpcError } from './errors';
export { createEventBuffer, subscribe, BROADCAST_EVENTS } from './events';
export type { BroadcastEventName, BufferedEvent } from './events';
export { getBackend, initBackend, isTauriRuntime, setBackend } from './backend';
export type { Backend, Unlisten, WindowControls } from './backend';
export type * from './types';
export { SECRET_PATHS } from './types';
