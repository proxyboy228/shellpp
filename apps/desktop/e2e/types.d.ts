// Type declarations shared by the e2e config, helpers and specs.

import type { E2eMockAction, E2eMockResult } from '../src/ipc/types.ts';

/** `tauri:options` capability understood by tauri-driver. */
export interface TauriOptions {
  /** Path of the application binary (tauri-driver adds `.exe` on Windows). */
  application: string;
  /** Command-line arguments for the application. */
  args?: string[];
  /** Windows only: passed to msedgedriver as `ms:edgeOptions.webviewOptions`. */
  webviewOptions?: Record<string, unknown>;
}

/**
 * What a terminal pane exposes to the specs: the shape of `TerminalProbe` in
 * src/ipc/e2e.ts (stage 2 plan, section 8.3). Declared here structurally so
 * the e2e program does not compile the frontend's modules.
 */
export interface E2eTerminalProbe {
  sessionId(): string | null;
  /** Scrollback and screen of the normal buffer, lines joined with `\n`, trailing spaces removed. */
  text(): string;
  selection(): string;
  renderer(): 'webgl' | 'dom';
  size(): { cols: number; rows: number };
}

declare global {
  namespace WebdriverIO {
    interface Capabilities {
      'tauri:options'?: TauriOptions;
    }
  }

  interface Window {
    /** Installed by `src/ipc/e2e.ts` in builds made with `vite build --mode e2e`. */
    __SHELLPP_E2E__?: {
      mock(action: E2eMockAction): Promise<E2eMockResult>;
      /** Pane ids with a registered terminal, in registration order. */
      terminalPanes(): string[];
      /** Text of a pane's terminal (the focused pane when omitted); null when there is none. */
      terminalText(paneId?: string): string | null;
      /** The probe of a pane (the focused pane when omitted). */
      terminal(paneId?: string): E2eTerminalProbe | null;
    };
  }
}
