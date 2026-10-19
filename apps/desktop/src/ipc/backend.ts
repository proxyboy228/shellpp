// The backend behind `src/ipc`: the Tauri runtime inside the app, or the
// in-browser mock for Vitest and `pnpm dev` without Tauri.
//
// `src/ipc/` is the only module that talks to Tauri (docs/architecture.md,
// section 2.5). The rest of the UI imports typed functions from `src/ipc`.
//
// Owner: W4.

import type { MockBackendOptions } from './mock';
import type { CommandInput, CommandName, CommandOutput, EventMap, EventName, IpcChannel } from './types';

export type Unlisten = () => void;

/** Title bar window controls (UI 1.1, 1.4 View > Full screen). */
export interface WindowControls {
  /** This window's label, `w-<n>`. */
  readonly label: string;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  isMaximized(): Promise<boolean>;
  close(): Promise<void>;
  startDragging(): Promise<void>;
  setFullscreen(fullscreen: boolean): Promise<void>;
  /** Called after the window was resized (maximize state may have changed). */
  onResized(handler: () => void): Promise<Unlisten>;
}

export interface Backend {
  readonly kind: 'tauri' | 'mock';
  /** Rejects with an `IpcError`. */
  invoke<C extends CommandName>(command: C, input: CommandInput<C>): Promise<CommandOutput<C>>;
  /** Broadcast events and events targeted at this window. */
  listen<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void): Promise<Unlisten>;
  /**
   * Creates a channel to pass inside a command input (`session_attach`).
   * Messages arrive in order; `close()` stops delivery to `onMessage`.
   */
  createChannel<T>(onMessage: (message: T) => void): IpcChannel<T> & { close(): void };
  readonly window: WindowControls;
}

let current: Backend | null = null;

/** True inside the Tauri webview. */
export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * Selects the backend once at startup: Tauri inside the app; the mock in
 * development and tests. A production build outside Tauri has no backend.
 */
export async function initBackend(): Promise<Backend> {
  if (current !== null) {
    return current;
  }
  if (isTauriRuntime()) {
    const { createTauriBackend } = await import('./tauri');
    current = createTauriBackend();
  } else if (import.meta.env.DEV || import.meta.env.MODE === 'test') {
    const { createMockBackend } = await import('./mock');
    current = createMockBackend(devMockOptions(window.location.search));
  } else {
    throw new Error('Shell++ must run inside the desktop app.');
  }
  return current;
}

/**
 * Options of the development mock from the page query (`pnpm dev` in a
 * browser): no query starts at first run with an empty vault; `?mock=demo`
 * starts unlocked with demo hosts and groups; `?mock=locked` starts locked
 * with the demo data; `?mock=first-run-demo` runs first run and then shows the
 * demo data. The demo master password is `shellpp-demo` (mock only, never a
 * real credential).
 */
export function devMockOptions(search: string): MockBackendOptions {
  const mode = new URLSearchParams(search).get('mock');
  const common = { kdfDelayMs: 300, timers: true } as const;
  if (mode === 'demo' || mode === 'locked') {
    return {
      ...common,
      initialState: mode === 'demo' ? 'unlocked' : 'locked',
      fixtures: 'demo',
      password: 'shellpp-demo',
    };
  }
  return { ...common, fixtures: mode === 'first-run-demo' ? 'demo' : 'empty' };
}

/** The selected backend. Call `initBackend` (or `setBackend` in tests) first. */
export function getBackend(): Backend {
  if (current === null) {
    throw new Error('The IPC backend is not initialized.');
  }
  return current;
}

/** Replaces the backend: for tests (`setBackend(createMockBackend())`) and teardown (`null`). */
export function setBackend(backend: Backend | null): void {
  current = backend;
}
