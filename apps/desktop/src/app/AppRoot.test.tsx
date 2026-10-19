// App window routing and the lock flows (UI 13.1-13.3) against the mock backend.

import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { HostsScreenProps } from '../features/hosts';
import { unlock } from '../ipc';
import { lockState, newPassword, renderApp, renderUnlocked, resetApp } from '../test/app';

const hostsStub = vi.hoisted(() => ({ fail: false }));

vi.mock('../features/hosts', () => ({
  HostsScreen: ({ vault, active, selectRequest }: HostsScreenProps) => {
    if (hostsStub.fail) {
      throw new Error('render failure');
    }
    return (
      <section
        data-testid="hosts-screen"
        data-vault-id={vault.vault_id}
        data-active={String(active)}
        data-select={selectRequest?.record_id ?? ''}
      />
    );
  },
}));

afterEach(() => {
  hostsStub.fail = false;
  resetApp();
});

describe('AppRoot: first run (UI 13.1)', () => {
  it('creates the master password, shows the recovery key and opens the empty Hosts screen', async () => {
    const { backend, user } = await renderApp();
    expect(lockState()).toBe('first_run');
    expect(screen.getByTestId('first-run-screen')).toBeInTheDocument();
    expect(screen.getByTestId('first-run-warning')).toBeVisible();
    expect(screen.queryByTestId('titlebar-menu-button')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-window-close')).toBeInTheDocument();

    const continueButton = screen.getByTestId('first-run-continue');
    expect(continueButton).toBeDisabled();
    expect(screen.getByTestId('first-run-password')).toHaveFocus();
    expect(screen.getByTestId('first-run-password')).toHaveAttribute('type', 'password');
    await user.click(screen.getByTestId('first-run-password-toggle'));
    expect(screen.getByTestId('first-run-password')).toHaveAttribute('type', 'text');

    await user.type(screen.getByTestId('first-run-password'), 'short');
    await user.type(screen.getByTestId('first-run-confirm'), 'shorx');
    expect(screen.getByTestId('first-run-confirm-error')).toBeInTheDocument();
    expect(continueButton).toBeDisabled();

    await user.clear(screen.getByTestId('first-run-confirm'));
    await user.type(screen.getByTestId('first-run-confirm'), 'short');
    expect(continueButton).toBeEnabled();
    await user.click(continueButton);
    expect(await screen.findByTestId('first-run-password-error')).toBeInTheDocument();
    expect(lockState()).toBe('first_run');

    const password = newPassword();
    await user.clear(screen.getByTestId('first-run-password'));
    await user.clear(screen.getByTestId('first-run-confirm'));
    await user.type(screen.getByTestId('first-run-password'), password);
    await user.type(screen.getByTestId('first-run-confirm'), password);
    await user.click(screen.getByTestId('first-run-continue'));

    const step = await screen.findByTestId('first-run-recovery-step');
    // The core is unlocked, but the window stays on the recovery key step.
    expect(lockState()).toBe('first_run');
    expect(within(step).getByTestId('first-run-warning')).toBeVisible();
    const key = screen.getByTestId('first-run-recovery-key').textContent ?? '';
    expect(key).toMatch(/^RK1(-[0-9A-Z]{5}){11}$/);
    expect(key).toBe(backend.control.recoveryKey());

    await user.click(screen.getByTestId('first-run-recovery-copy'));
    expect(await screen.findByTestId('first-run-copied-toast')).toBeInTheDocument();
    expect(backend.control.clipboard()).toBe(key);
    // The core runs unlocked during this step: input counts for auto-lock (UI 13.3).
    expect(backend.control.activityCount()).toBeGreaterThan(0);

    const finish = screen.getByTestId('first-run-finish');
    expect(finish).toBeDisabled();
    await user.click(screen.getByTestId('first-run-recovery-saved'));
    expect(finish).toBeEnabled();
    await user.click(finish);

    await waitFor(() => expect(lockState()).toBe('unlocked'));
    expect(screen.getByTestId('vaults-tab')).toBeInTheDocument();
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-vault-id');
    expect(screen.getByTestId('titlebar-menu-button')).toBeInTheDocument();
  });
});

describe('AppRoot: unlock (UI 13.2)', () => {
  it('shows the unlock screen with the field focused; a wrong password is cleared and refused', async () => {
    const password = newPassword();
    const { user } = await renderApp({ initialState: 'locked', password, fixtures: 'demo' });
    expect(lockState()).toBe('locked');
    expect(screen.getByTestId('unlock-app-name')).toHaveTextContent('Shell++');
    expect(screen.queryByTestId('vaults-tab')).not.toBeInTheDocument();
    expect(screen.queryByTestId('titlebar-tab-vaults')).not.toBeInTheDocument();
    const field = screen.getByTestId('unlock-password');
    expect(field).toHaveFocus();

    await user.type(field, `${newPassword()}{Enter}`);
    expect(await screen.findByTestId('unlock-password-error')).toBeInTheDocument();
    expect(field).toHaveValue('');
    expect(field).toHaveFocus();

    await user.type(field, `${password}{Enter}`);
    await waitFor(() => expect(lockState()).toBe('unlocked'));
    expect(screen.getByTestId('hosts-screen')).toBeInTheDocument();
  });

  it('unlocks with the recovery key and a new password', async () => {
    const { backend, user } = await renderApp({ initialState: 'locked', password: newPassword() });
    await user.click(screen.getByTestId('unlock-use-recovery-key'));
    expect(screen.getByTestId('recovery-unlock-screen')).toBeInTheDocument();
    expect(screen.getByTestId('recovery-unlock-key')).toHaveFocus();

    const next = newPassword();
    await user.type(screen.getByTestId('recovery-unlock-key'), 'RK1-WRONG');
    await user.type(screen.getByTestId('recovery-unlock-password'), next);
    await user.type(screen.getByTestId('recovery-unlock-confirm'), `${next}x`);
    expect(screen.getByTestId('recovery-unlock-confirm-error')).toBeInTheDocument();
    expect(screen.getByTestId('recovery-unlock-submit')).toBeDisabled();
    await user.type(screen.getByTestId('recovery-unlock-confirm'), '{Backspace}');
    await user.click(screen.getByTestId('recovery-unlock-submit'));
    expect(await screen.findByTestId('recovery-unlock-key-error')).toBeInTheDocument();

    await user.clear(screen.getByTestId('recovery-unlock-key'));
    await user.type(screen.getByTestId('recovery-unlock-key'), backend.control.recoveryKey() ?? '');
    await user.click(screen.getByTestId('recovery-unlock-submit'));
    await waitFor(() => expect(lockState()).toBe('unlocked'));

    // The new password works after the next lock; the window returns to the password screen.
    await act(async () => {
      backend.control.autoLock();
    });
    await waitFor(() => expect(lockState()).toBe('locked'));
    expect(screen.getByTestId('unlock-screen')).toBeInTheDocument();
    await user.type(screen.getByTestId('unlock-password'), `${next}{Enter}`);
    await waitFor(() => expect(lockState()).toBe('unlocked'));
  });

  it('shows the unlock screen after the next lock when another window unlocked meanwhile', async () => {
    const password = newPassword();
    const { backend, user } = await renderApp({ initialState: 'locked', password });
    await user.click(screen.getByTestId('unlock-use-recovery-key'));
    expect(screen.getByTestId('recovery-unlock-screen')).toBeInTheDocument();
    // Another window unlocks with the master password.
    await act(async () => {
      await unlock(password);
    });
    await waitFor(() => expect(lockState()).toBe('unlocked'));
    await act(async () => {
      backend.control.autoLock();
    });
    await waitFor(() => expect(lockState()).toBe('locked'));
    expect(screen.getByTestId('unlock-screen')).toBeInTheDocument();
    expect(screen.queryByTestId('recovery-unlock-screen')).not.toBeInTheDocument();
    expect(screen.getByTestId('unlock-password')).toHaveFocus();
  });

  it('goes back from the recovery screen to the password', async () => {
    const { user } = await renderApp({ initialState: 'locked', password: newPassword() });
    await user.click(screen.getByTestId('unlock-use-recovery-key'));
    await user.click(screen.getByTestId('recovery-unlock-back'));
    expect(screen.getByTestId('unlock-screen')).toBeInTheDocument();
  });
});

describe('AppRoot: lock and auto-lock (UI 13.3)', () => {
  it('opens and closes Settings, then locks from the app menu and hides the content', async () => {
    const { user, password } = await renderUnlocked();
    await user.click(screen.getByTestId('titlebar-menu-button'));
    await user.click(screen.getByTestId('app-menu-settings'));
    expect(await screen.findByTestId('settings-modal')).toBeInTheDocument();
    await user.click(screen.getByTestId('settings-modal-close'));
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument();

    await user.keyboard('{Control>},{/Control}');
    expect(await screen.findByTestId('settings-modal')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('titlebar-menu-button'));
    await user.click(screen.getByTestId('app-menu-lock'));
    await waitFor(() => expect(lockState()).toBe('locked'));
    expect(screen.queryByTestId('vaults-tab')).not.toBeInTheDocument();
    expect(screen.queryByTestId('app-menu')).not.toBeInTheDocument();

    await user.type(screen.getByTestId('unlock-password'), `${password}{Enter}`);
    await waitFor(() => expect(lockState()).toBe('unlocked'));
    expect(screen.getByTestId('hosts-screen')).toBeInTheDocument();
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument();
  });

  it('reopens the Settings window that was open when the app locked', async () => {
    const { backend, user, password } = await renderUnlocked();
    await user.keyboard('{Control>},{/Control}');
    expect(await screen.findByTestId('settings-modal')).toBeInTheDocument();
    await act(async () => {
      backend.control.autoLock();
    });
    await waitFor(() => expect(lockState()).toBe('locked'));
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument();
    await user.type(screen.getByTestId('unlock-password'), `${password}{Enter}`);
    await waitFor(() => expect(lockState()).toBe('unlocked'));
    expect(await screen.findByTestId('settings-modal')).toBeInTheDocument();
  });

  it('auto-locks every view, drops toasts and keeps the window buttons', async () => {
    const { backend } = await renderUnlocked();
    await act(async () => {
      backend.control.notify('notifications.sample');
    });
    expect(await screen.findByTestId('notification-toast')).toBeInTheDocument();
    await act(async () => {
      backend.control.autoLock();
    });
    await waitFor(() => expect(lockState()).toBe('locked'));
    expect(screen.queryByTestId('notification-toast')).not.toBeInTheDocument();
    expect(screen.queryByTestId('vaults-tab')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-window-minimize')).toBeInTheDocument();
    expect(screen.queryByTestId('titlebar-notifications-button')).not.toBeInTheDocument();
  });
});

describe('AppRoot: boot', () => {
  it('shows the startup error screen when the core could not open', async () => {
    await renderApp({ startupError: 'newer_schema' });
    expect(lockState()).toBe('error');
    expect(screen.getByTestId('app-fatal-error')).toHaveTextContent('newer version');
  });

  it('labels the window and its OS', async () => {
    await renderUnlocked({ os: 'linux', windowLabel: 'w-3' });
    const root = screen.getByTestId('app-root');
    expect(root).toHaveAttribute('data-window-label', 'w-3');
    expect(root).toHaveAttribute('data-os', 'linux');
  });

  it('passes a host link to the Hosts screen', async () => {
    const { backend } = await renderUnlocked();
    const recordId = crypto.randomUUID();
    await act(async () => {
      backend.control.emitLink(recordId);
    });
    await waitFor(() => expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-select', recordId));
  });

  it('marks the Hosts screen inactive while Settings is open', async () => {
    const { user } = await renderUnlocked();
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-active', 'true');
    await user.keyboard('{Control>},{/Control}');
    await screen.findByTestId('settings-modal');
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-active', 'false');
  });

  it('applies the interface theme and zoom from the settings', async () => {
    await renderUnlocked({ settings: { 'appearance.ui_theme': 'light', 'appearance.ui_zoom': 1.2 } });
    expect(document.documentElement).toHaveAttribute('data-theme', 'light');
    expect(document.documentElement.style.getPropertyValue('zoom')).toBe('1.2');
  });
});

describe('AppRoot: rendering errors', () => {
  it('shows a reload message instead of a blank window and keeps the title bar', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    hostsStub.fail = true;
    await renderUnlocked();
    expect(screen.getByTestId('app-render-error')).toBeInTheDocument();
    expect(screen.getByTestId('app-render-error-reload')).toBeInTheDocument();
    expect(screen.getByTestId('titlebar-menu-button')).toBeInTheDocument();
    consoleError.mockRestore();
  });
});
