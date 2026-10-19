// Test helpers for the app shell: a fresh mock backend and window stores per
// test, and the rendered `<AppRoot/>`. Passwords are generated at run time.
//
// Owner: lead.

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect } from 'vitest';
import { AppRoot } from '../app/AppRoot';
import { resetStores } from '../app/store';
import { resetWorkspace } from '../features/terminal';
import { clearToasts } from '../app/toast';
import { setBackend } from '../ipc';
import { createMockBackend, type MockBackend, type MockBackendOptions } from '../ipc/mock';

/** A password made at run time (never a fixed credential). */
export function newPassword(): string {
  return `pw-${crypto.randomUUID().slice(0, 13)}`;
}

/** Fresh stores and a new mock backend; call `resetApp()` after the test. */
export function setupBackend(options: MockBackendOptions = {}): MockBackend {
  resetStores();
  resetWorkspace();
  clearToasts();
  const backend = createMockBackend(options);
  setBackend(backend);
  return backend;
}

export function resetApp(): void {
  // Unmount first: the stores below notify mounted components.
  cleanup();
  setBackend(null);
  resetStores();
  resetWorkspace();
  clearToasts();
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.style.removeProperty('zoom');
}

/** Renders the app window and waits until it left the booting state. */
export async function renderApp(options: MockBackendOptions = {}) {
  const backend = setupBackend(options);
  const user = userEvent.setup();
  const result = render(<AppRoot />);
  await waitFor(() => expect(screen.getByTestId('app-root')).not.toHaveAttribute('data-lock-state', 'booting'));
  return { backend, user, ...result };
}

/** The `data-lock-state` of the app root. */
export function lockState(): string | null {
  return screen.getByTestId('app-root').getAttribute('data-lock-state');
}

/** Renders the app unlocked with a generated password. */
export async function renderUnlocked(options: MockBackendOptions = {}) {
  const password = newPassword();
  const rendered = await renderApp({ initialState: 'unlocked', password, ...options });
  await waitFor(() => expect(lockState()).toBe('unlocked'));
  return { ...rendered, password };
}
