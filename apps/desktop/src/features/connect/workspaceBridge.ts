// The terminal workspace actions the S7 features call (features/terminal,
// S6): open a session or a local terminal, activate a tab, restore the last
// session. One indirection, called at run time, so unit tests can replace
// these actions without mocking the terminal module itself (it imports the
// connect and new tab features back, and a mock of a module in an import
// cycle does not reach every importer).
//
// Owner: S7.

import * as terminal from '../terminal';
import type { OpenOptions, OpenResult, TabId } from '../terminal';
import type { SessionTarget } from '../../ipc';

export function openSession(target: SessionTarget, options: OpenOptions = {}): Promise<OpenResult | null> {
  return terminal.openSession(target, options);
}

export function openLocalTerminal(options: OpenOptions & { shellId?: string | null } = {}): Promise<OpenResult | null> {
  return terminal.openLocalTerminal(options);
}

export function activateTab(id: 'vaults' | TabId): void {
  terminal.activateTab(id);
}

export function restoreLastSession(): Promise<void> {
  return terminal.restoreLastSession();
}
