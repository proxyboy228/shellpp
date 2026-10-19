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
    };
  }
}
