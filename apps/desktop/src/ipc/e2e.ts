// e2e helper (docs/architecture.md, section 5.4): in builds made with
// `vite build --mode e2e` the page exposes `window.__SHELLPP_E2E__`, which the
// WebdriverIO specs call through `browser.execute`. Production builds do not
// contain it: the call site is guarded by `import.meta.env.MODE === 'e2e'`.
//
// Stage 2: terminal probes. xterm.js draws with WebGL (or its DOM renderer),
// so specs read terminal text through probes that the terminal views
// register in e2e builds only (features/terminal, stage 2 plan, section 8.3).
//
// Owner: lead (used by S6 and S8).

import { e2eMock } from './commands';
import type { E2eMockAction, E2eMockResult } from './types';

/** What a terminal pane exposes to e2e specs. */
export interface TerminalProbe {
  /** Session shown in the pane. */
  sessionId(): string | null;
  /** Scrollback and screen of the normal buffer, lines joined with `\n`, trailing spaces removed. */
  text(): string;
  /** The current selection text. */
  selection(): string;
  /** `webgl` or `dom`. */
  renderer(): 'webgl' | 'dom';
  /** Columns and rows. */
  size(): { cols: number; rows: number };
}

export interface ShellppE2eHelper {
  /** Calls the `e2e_mock` command. */
  mock(action: E2eMockAction): Promise<E2eMockResult>;
  /** Pane ids with a registered terminal, in registration order. */
  terminalPanes(): string[];
  /** Text of a pane's terminal (the active pane when omitted); null when there is none. */
  terminalText(paneId?: string): string | null;
  /** The probe of a pane (the active pane when omitted). */
  terminal(paneId?: string): TerminalProbe | null;
}

declare global {
  interface Window {
    __SHELLPP_E2E__?: ShellppE2eHelper;
  }
}

const probes = new Map<string, TerminalProbe>();
let activePaneId: string | null = null;

/**
 * Registers a terminal pane (e2e builds only: call it only when
 * `import.meta.env.MODE === 'e2e'`). Returns a function that unregisters it.
 */
export function registerTerminalProbe(paneId: string, probe: TerminalProbe): () => void {
  probes.set(paneId, probe);
  return () => {
    if (probes.get(paneId) === probe) {
      probes.delete(paneId);
    }
    if (activePaneId === paneId) {
      activePaneId = null;
    }
  };
}

/** The pane whose terminal has focus or was focused last (e2e builds only). */
export function setActiveTerminalProbe(paneId: string | null): void {
  activePaneId = paneId;
}

function probeOf(paneId?: string): TerminalProbe | null {
  const id = paneId ?? activePaneId;
  return id === null ? null : (probes.get(id) ?? null);
}

/** Installs the helper. Call only when `import.meta.env.MODE === 'e2e'`. */
export function installE2eHelper(): void {
  window.__SHELLPP_E2E__ = {
    mock: (action) => e2eMock(action),
    terminalPanes: () => [...probes.keys()],
    terminalText: (paneId) => probeOf(paneId)?.text() ?? null,
    terminal: (paneId) => probeOf(paneId),
  };
}
