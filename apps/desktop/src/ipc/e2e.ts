// e2e helper (docs/architecture.md, section 5.4): in builds made with
// `vite build --mode e2e` the page exposes `window.__SHELLPP_E2E__`, which the
// WebdriverIO specs call through `browser.execute`. Production builds do not
// contain it: the call site is guarded by `import.meta.env.MODE === 'e2e'`.
//
// Owner: W4 (used by W6).

import { e2eMock } from './commands';
import type { E2eMockAction, E2eMockResult } from './types';

export interface ShellppE2eHelper {
  /** Calls the `e2e_mock` command. */
  mock(action: E2eMockAction): Promise<E2eMockResult>;
}

declare global {
  interface Window {
    __SHELLPP_E2E__?: ShellppE2eHelper;
  }
}

/** Installs the helper. Call only when `import.meta.env.MODE === 'e2e'`. */
export function installE2eHelper(): void {
  window.__SHELLPP_E2E__ = {
    mock: (action) => e2eMock(action),
  };
}
