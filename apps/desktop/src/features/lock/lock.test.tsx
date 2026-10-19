// Lock screen details (UI 13.1-13.2) not covered by the AppRoot flows.

import { screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { newPassword, renderApp, resetApp } from '../../test/app';

vi.mock('../hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
});

describe('lock screens', () => {
  it('asks to show the key again when the copy grant expired', async () => {
    const { backend, user } = await renderApp();
    const password = newPassword();
    await user.type(screen.getByTestId('first-run-password'), password);
    await user.type(screen.getByTestId('first-run-confirm'), `${password}{Enter}`);
    await screen.findByTestId('first-run-recovery-step');
    backend.control.advanceClock(6 * 60_000);
    await user.click(screen.getByTestId('first-run-recovery-copy'));
    expect(await screen.findByTestId('app-error-toast')).toHaveTextContent('show it again');
    expect(backend.control.clipboard()).toBeNull();
  });

  it('copies the selected recovery key only through the protected copy', async () => {
    const { backend, user } = await renderApp();
    const password = newPassword();
    await user.type(screen.getByTestId('first-run-password'), password);
    await user.type(screen.getByTestId('first-run-confirm'), `${password}{Enter}`);
    await screen.findByTestId('first-run-recovery-step');
    const key = screen.getByTestId('first-run-recovery-key');
    expect(key).toHaveAttribute('data-sensitive');
    const event = new Event('copy', { bubbles: true, cancelable: true });
    key.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    await waitFor(() => expect(backend.control.clipboard()).toBe(key.textContent));
    expect(backend.control.calls()).toContain('secret_copy');
    expect(backend.control.calls()).not.toContain('clipboard_write_text');
  });

  it('keeps Unlock disabled while the field is empty and does not send an empty password', async () => {
    const { backend, user } = await renderApp({ initialState: 'locked', password: newPassword() });
    expect(screen.getByTestId('unlock-submit')).toBeDisabled();
    await user.keyboard('{Enter}');
    expect(backend.control.calls()).not.toContain('unlock');
  });

  it('shows the hidden password on demand on the unlock screen', async () => {
    const { user } = await renderApp({ initialState: 'locked', password: newPassword() });
    const field = screen.getByTestId('unlock-password');
    expect(field).toHaveAttribute('type', 'password');
    await user.click(screen.getByTestId('unlock-password-toggle'));
    expect(field).toHaveAttribute('type', 'text');
  });

  it('refuses a malformed recovery key and a short new password together', async () => {
    const { user } = await renderApp({ initialState: 'locked', password: newPassword() });
    await user.click(screen.getByTestId('unlock-use-recovery-key'));
    await user.type(screen.getByTestId('recovery-unlock-key'), 'not a key');
    await user.type(screen.getByTestId('recovery-unlock-password'), 'short');
    await user.type(screen.getByTestId('recovery-unlock-confirm'), 'short{Enter}');
    await waitFor(() => expect(screen.getByTestId('recovery-unlock-key-error')).toBeInTheDocument());
    expect(screen.getByTestId('recovery-unlock-password-error')).toBeInTheDocument();
  });
});
