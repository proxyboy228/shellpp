// Settings window (UI 12.1) and Security (UI 13.4) against the mock backend.

import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CHANGELOG_URL, DOCUMENTATION_URL, ISSUES_URL } from '../../app/links';
import { lockState, newPassword, renderUnlocked, resetApp } from '../../test/app';

vi.mock('../hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
});

type User = Awaited<ReturnType<typeof renderUnlocked>>['user'];

async function openSettings(user: User) {
  await user.keyboard('{Control>},{/Control}');
  return screen.findByTestId('settings-modal');
}

describe('Settings window', () => {
  it('shows only Security, the external links and the version', async () => {
    const { backend, user } = await renderUnlocked({ version: '0.1.0' });
    await openSettings(user);
    const nav = screen.getByTestId('settings-nav');
    expect(within(nav).getAllByRole('button').map((button) => button.getAttribute('data-testid'))).toEqual([
      'settings-nav-security',
      'settings-link-documentation',
      'settings-link-report-bug',
      'settings-link-changelog',
    ]);
    expect(screen.getByTestId('settings-nav-security')).toHaveAttribute('aria-current', 'page');
    for (const id of [
      'settings-nav-account-sync',
      'settings-nav-ssh-id',
      'settings-nav-team',
      'settings-nav-terminal',
      'settings-nav-sftp',
      'settings-nav-logs',
      'settings-nav-shortcuts',
      'settings-nav-updates',
      'settings-theme-select',
      'security-biometric',
    ]) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
    expect(screen.getByTestId('settings-version')).toHaveTextContent('0.1.0');
    expect(screen.getByTestId('settings-section-security')).toBeInTheDocument();

    await user.click(screen.getByTestId('settings-link-documentation'));
    await user.click(screen.getByTestId('settings-link-report-bug'));
    await user.click(screen.getByTestId('settings-link-changelog'));
    await waitFor(() => expect(backend.control.openedUrls()).toEqual([DOCUMENTATION_URL, ISSUES_URL, CHANGELOG_URL]));
  });

  it('changes the auto-lock interval at once, including Never', async () => {
    const { backend, user } = await renderUnlocked();
    await openSettings(user);
    const select = screen.getByTestId('security-auto-lock');
    expect(select).toHaveValue('15');
    expect(within(select).getAllByRole('option').map((option) => option.getAttribute('value'))).toEqual([
      '1',
      '5',
      '15',
      '30',
      '60',
      'never',
    ]);
    await user.selectOptions(select, '1');
    await waitFor(async () =>
      expect((await backend.invoke('settings_get', undefined))['security.auto_lock_minutes']).toBe(1),
    );
    await user.selectOptions(select, 'never');
    await waitFor(async () =>
      expect((await backend.invoke('settings_get', undefined))['security.auto_lock_minutes']).toBeNull(),
    );
    expect(select).toHaveValue('never');
    // Close and reopen: the value stays.
    await user.keyboard('{Escape}');
    await openSettings(user);
    expect(screen.getByTestId('security-auto-lock')).toHaveValue('never');
  });

  it('changes the master password with errors under the fields', async () => {
    const { backend, user, password } = await renderUnlocked();
    await openSettings(user);
    await user.click(screen.getByTestId('security-change-password'));
    const dialog = screen.getByTestId('change-password-dialog');
    expect(screen.getByTestId('change-password-current')).toHaveFocus();
    expect(screen.queryByTestId('change-password-sync-warning')).not.toBeInTheDocument();
    const submit = within(dialog).getByTestId('change-password-submit');
    expect(submit).toBeDisabled();

    const next = newPassword();
    await user.type(screen.getByTestId('change-password-current'), newPassword());
    await user.type(screen.getByTestId('change-password-new'), next);
    await user.type(screen.getByTestId('change-password-confirm'), `${next}!`);
    expect(screen.getByTestId('change-password-confirm-error')).toBeInTheDocument();
    expect(submit).toBeDisabled();
    await user.type(screen.getByTestId('change-password-confirm'), '{Backspace}');
    await user.click(submit);
    expect(await screen.findByTestId('change-password-current-error')).toBeInTheDocument();
    expect(screen.getByTestId('change-password-current')).toHaveValue('');

    await user.type(screen.getByTestId('change-password-current'), password);
    await user.clear(screen.getByTestId('change-password-new'));
    await user.clear(screen.getByTestId('change-password-confirm'));
    await user.type(screen.getByTestId('change-password-new'), 'short');
    await user.type(screen.getByTestId('change-password-confirm'), 'short');
    await user.click(submit);
    expect(await screen.findByTestId('change-password-new-error')).toBeInTheDocument();

    await user.clear(screen.getByTestId('change-password-new'));
    await user.clear(screen.getByTestId('change-password-confirm'));
    await user.type(screen.getByTestId('change-password-new'), next);
    await user.type(screen.getByTestId('change-password-confirm'), next);
    await user.click(submit);
    expect(await screen.findByTestId('security-password-changed-toast')).toBeInTheDocument();
    expect(screen.queryByTestId('change-password-dialog')).not.toBeInTheDocument();
    expect(screen.getByTestId('settings-modal')).toBeInTheDocument();

    await act(async () => {
      backend.control.autoLock();
    });
    await waitFor(() => expect(lockState()).toBe('locked'));
    await user.type(screen.getByTestId('unlock-password'), `${next}{Enter}`);
    await waitFor(() => expect(lockState()).toBe('unlocked'));
  });

  it('shows the recovery key after the password', async () => {
    const { backend, user, password } = await renderUnlocked();
    await openSettings(user);
    await user.click(screen.getByTestId('security-recovery-show'));
    expect(screen.getByTestId('recovery-password-dialog')).toBeInTheDocument();
    await user.type(screen.getByTestId('recovery-password-input'), `${newPassword()}{Enter}`);
    expect(await screen.findByTestId('recovery-password-input-error')).toBeInTheDocument();
    await user.type(screen.getByTestId('recovery-password-input'), `${password}{Enter}`);
    expect(await screen.findByTestId('recovery-key-dialog')).toBeInTheDocument();
    expect(screen.getByTestId('recovery-key-text')).toHaveTextContent(backend.control.recoveryKey() ?? 'missing');
    expect(screen.queryByTestId('recovery-key-saved')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('recovery-key-copy'));
    await waitFor(() => expect(backend.control.clipboard()).toBe(backend.control.recoveryKey()));
    expect(screen.getByTestId('recovery-key-done')).toBeEnabled();
    await user.click(screen.getByTestId('recovery-key-done'));
    expect(screen.queryByTestId('recovery-key-dialog')).not.toBeInTheDocument();
  });

  it('reissues the recovery key with the required checkbox', async () => {
    const { backend, user, password } = await renderUnlocked();
    const before = backend.control.recoveryKey();
    await openSettings(user);
    await user.click(screen.getByTestId('security-recovery-reissue'));
    await user.type(screen.getByTestId('recovery-password-input'), password);
    await user.click(screen.getByTestId('recovery-password-submit'));
    expect(await screen.findByTestId('recovery-key-dialog')).toBeInTheDocument();
    const key = screen.getByTestId('recovery-key-text').textContent;
    expect(key).not.toBe(before);
    expect(key).toBe(backend.control.recoveryKey());
    const done = screen.getByTestId('recovery-key-done');
    expect(done).toBeDisabled();
    // Escape does not close it before the checkbox is checked.
    await user.keyboard('{Escape}');
    expect(screen.getByTestId('recovery-key-dialog')).toBeInTheDocument();
    await user.click(screen.getByTestId('recovery-key-saved'));
    await user.click(done);
    expect(screen.queryByTestId('recovery-key-dialog')).not.toBeInTheDocument();
    expect(screen.getByTestId('settings-modal')).toBeInTheDocument();
  });

  it('turns remember on this device on after the warning and off directly', async () => {
    const { user } = await renderUnlocked();
    await openSettings(user);
    const toggle = screen.getByTestId('security-remember-device');
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    await user.click(toggle);
    expect(screen.getByTestId('remember-device-warning-dialog')).toBeInTheDocument();
    await user.click(screen.getByTestId('remember-device-warning-cancel'));
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    await user.click(toggle);
    await user.click(screen.getByTestId('remember-device-warning-confirm'));
    await waitFor(() => expect(screen.getByTestId('security-remember-device')).toHaveAttribute('aria-checked', 'true'));
    await user.click(screen.getByTestId('security-remember-device'));
    expect(screen.queryByTestId('remember-device-warning-dialog')).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('security-remember-device')).toHaveAttribute('aria-checked', 'false'));
  });

  it('keeps the lock state when the remember response arrives after a lock', async () => {
    const { backend, user } = await renderUnlocked();
    await openSettings(user);
    // Hold the `remember_device_set` answer (made while unlocked) until after the lock.
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const invoke = backend.invoke.bind(backend);
    backend.invoke = (async (command: string, input: unknown) => {
      const output = await (invoke as (c: string, i: unknown) => Promise<unknown>)(command, input);
      if (command === 'remember_device_set') {
        await gate;
      }
      return output;
    }) as typeof backend.invoke;
    await user.click(screen.getByTestId('security-remember-device'));
    await user.click(screen.getByTestId('remember-device-warning-confirm'));
    await act(async () => {
      backend.control.autoLock();
    });
    await waitFor(() => expect(lockState()).toBe('locked'));
    await act(async () => {
      release();
      await gate;
    });
    expect(lockState()).toBe('locked');
    expect(screen.getByTestId('unlock-screen')).toBeInTheDocument();
  });

  it('hides remember on this device without an OS keychain', async () => {
    const { user } = await renderUnlocked({ rememberDeviceAvailable: false });
    await openSettings(user);
    expect(screen.getByTestId('settings-section-security')).toBeInTheDocument();
    expect(screen.queryByTestId('security-remember-device')).not.toBeInTheDocument();
  });

  it('closes with the X button and Escape', async () => {
    const { user } = await renderUnlocked();
    await openSettings(user);
    await user.click(screen.getByTestId('settings-modal-close'));
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument();
    await openSettings(user);
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument();
  });
});
