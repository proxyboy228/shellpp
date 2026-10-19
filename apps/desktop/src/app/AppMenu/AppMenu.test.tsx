// App menu (UI 1.4): items in order, hidden items, submenus and actions.

import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DOCUMENTATION_URL, ISSUES_URL } from '../links';
import { lockState, renderUnlocked, resetApp } from '../../test/app';
import { buildAppMenu } from './AppMenu';

vi.mock('../../features/hosts', () => ({
  HostsScreen: () => (
    <section data-testid="hosts-screen">
      <input data-testid="test-field-a" aria-label="Field A" />
      <input data-testid="test-field-b" aria-label="Field B" />
      {/* A revealed secret: type text, marked sensitive (as PasswordInput). */}
      <input data-testid="test-field-secret" aria-label="Secret" data-sensitive="" />
    </section>
  ),
}));

afterEach(() => {
  resetApp();
  vi.unstubAllGlobals();
});

function topLevelIds(): string[] {
  const menu = screen.getByTestId('app-menu');
  return within(menu)
    .getAllByRole('menuitem')
    .filter((item) => item.closest('[role="menu"]') === menu)
    .map((item) => item.getAttribute('data-testid') ?? '');
}

async function openMenu(user: Awaited<ReturnType<typeof renderUnlocked>>['user']) {
  await user.click(screen.getByTestId('titlebar-menu-button'));
  return screen.getByTestId('app-menu');
}

async function openSubmenu(user: Awaited<ReturnType<typeof renderUnlocked>>['user'], id: string) {
  await openMenu(user);
  await user.click(screen.getByTestId(id));
  return screen.getByTestId(`${id}-menu`);
}

describe('App menu', () => {
  it('lists the stage-2 items in order with shortcut hints', async () => {
    const { user } = await renderUnlocked();
    await openMenu(user);
    expect(topLevelIds()).toEqual([
      'app-menu-vaults',
      'app-menu-settings',
      'app-menu-new-local-terminal',
      'app-menu-edit',
      'app-menu-view',
      'app-menu-window',
      'app-menu-help',
      'app-menu-lock',
      'app-menu-exit',
    ]);
    expect(screen.getByTestId('app-menu-vaults')).toHaveAttribute('aria-keyshortcuts', 'Ctrl+1');
    expect(screen.getByTestId('app-menu-settings')).toHaveAttribute('aria-keyshortcuts', 'Ctrl+,');
    expect(screen.getByTestId('app-menu-new-local-terminal')).toHaveAttribute('aria-keyshortcuts', 'Ctrl+L');
    for (const id of ['app-menu-sftp', 'app-menu-new-serial']) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
    expect(within(screen.getByTestId('app-menu')).getAllByRole('separator')).toHaveLength(2);
  });

  it('writes Cmd in the hints on macOS', () => {
    const entries = buildAppMenu('macos');
    const settings = entries.find((entry) => entry.id === 'app-menu-settings');
    expect(settings && 'shortcut' in settings ? settings.shortcut : null).toBe('Cmd+,');
  });

  it('has the Edit, View, Window and Help submenus without later-stage items', async () => {
    const { user } = await renderUnlocked();
    const edit = await openSubmenu(user, 'app-menu-edit');
    for (const id of ['undo', 'redo', 'cut', 'copy', 'paste', 'select-all']) {
      expect(within(edit).getByTestId(`app-menu-edit-${id}`)).toBeInTheDocument();
    }
    await user.keyboard('{Escape}{Escape}');

    const view = await openSubmenu(user, 'app-menu-view');
    for (const id of ['zoom-in', 'zoom-out', 'zoom-reset', 'fullscreen', 'toggle-sidebar']) {
      expect(within(view).getByTestId(`app-menu-view-${id}`)).toBeInTheDocument();
    }
    await user.keyboard('{Escape}{Escape}');

    const windowMenu = await openSubmenu(user, 'app-menu-window');
    expect(within(windowMenu).getByTestId('app-menu-window-new-window')).toBeInTheDocument();
    for (const id of ['next-tab', 'previous-tab', 'close-tab']) {
      expect(within(windowMenu).getByTestId(`app-menu-window-${id}`)).toBeInTheDocument();
    }
    await user.keyboard('{Escape}{Escape}');

    const help = await openSubmenu(user, 'app-menu-help');
    for (const id of ['documentation', 'report-issue', 'about']) {
      expect(within(help).getByTestId(`app-menu-help-${id}`)).toBeInTheDocument();
    }
    expect(screen.queryByTestId('app-menu-help-check-updates')).not.toBeInTheDocument();
  });

  it('opens the help links in the browser and shows About with the version', async () => {
    const { backend, user } = await renderUnlocked({ version: '1.2.3' });
    await openSubmenu(user, 'app-menu-help');
    await user.click(screen.getByTestId('app-menu-help-documentation'));
    expect(screen.queryByTestId('app-menu')).not.toBeInTheDocument();
    await openSubmenu(user, 'app-menu-help');
    await user.click(screen.getByTestId('app-menu-help-report-issue'));
    await waitFor(() => expect(backend.control.openedUrls()).toEqual([DOCUMENTATION_URL, ISSUES_URL]));

    await openSubmenu(user, 'app-menu-help');
    await user.click(screen.getByTestId('app-menu-help-about'));
    expect(await screen.findByTestId('about-dialog')).toBeInTheDocument();
    expect(screen.getByTestId('about-dialog-version')).toHaveTextContent('1.2.3');
    await user.click(screen.getByTestId('about-dialog-close'));
    expect(screen.queryByTestId('about-dialog')).not.toBeInTheDocument();
  });

  it('zooms in, out and back through the ui_zoom setting', async () => {
    const { user } = await renderUnlocked();
    const zoom = () => document.documentElement.style.getPropertyValue('zoom');
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-zoom-in'));
    await waitFor(() => expect(zoom()).toBe('1.1'));
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-zoom-out'));
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-zoom-out'));
    await waitFor(() => expect(zoom()).toBe('0.9'));
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-zoom-reset'));
    await waitFor(() => expect(zoom()).toBe('1'));
  });

  it('toggles full screen and the sidebar', async () => {
    const { backend, user } = await renderUnlocked();
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-fullscreen'));
    await waitFor(() => expect(backend.control.windowState().fullscreen).toBe(true));

    expect(screen.getByTestId('sidebar')).toBeInTheDocument();
    expect(screen.getByTestId('sidebar-hosts')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('sidebar-known-hosts')).toBeInTheDocument();
    for (const id of ['sidebar-keychain', 'sidebar-port-forwarding', 'sidebar-snippets', 'sidebar-logs']) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-toggle-sidebar'));
    expect(screen.queryByTestId('sidebar')).not.toBeInTheDocument();
    await openSubmenu(user, 'app-menu-view');
    await user.click(screen.getByTestId('app-menu-view-toggle-sidebar'));
    expect(screen.getByTestId('sidebar')).toBeInTheDocument();
  });

  it('opens a new window, locks and exits', async () => {
    const { backend, user } = await renderUnlocked();
    await openSubmenu(user, 'app-menu-window');
    await user.click(screen.getByTestId('app-menu-window-new-window'));
    await waitFor(() => expect(backend.control.windowsOpened()).toEqual(['w-2']));

    await openMenu(user);
    await user.click(screen.getByTestId('app-menu-exit'));
    await waitFor(() => expect(backend.control.exitRequested()).toBe(true));

    await openMenu(user);
    await user.click(screen.getByTestId('app-menu-lock'));
    await waitFor(() => expect(lockState()).toBe('locked'));
  });

  it('copies and pastes between fields through the core clipboard', async () => {
    const execCommand = vi.fn((command: string, _ui?: boolean, value?: string) => {
      const target = document.activeElement;
      if (command === 'insertText' && target instanceof HTMLInputElement) {
        target.setRangeText(value ?? '', target.selectionStart ?? 0, target.selectionEnd ?? 0, 'end');
      }
      return true;
    });
    Object.defineProperty(document, 'execCommand', { configurable: true, value: execCommand });
    const { backend, user } = await renderUnlocked();

    const fieldA = screen.getByTestId('test-field-a') as HTMLInputElement;
    await user.type(fieldA, 'web-1.example.com');
    fieldA.setSelectionRange(0, 5);
    await openSubmenu(user, 'app-menu-edit');
    await user.click(screen.getByTestId('app-menu-edit-copy'));
    await waitFor(() => expect(backend.control.clipboard()).toBe('web-1'));

    await user.click(screen.getByTestId('test-field-b'));
    await openSubmenu(user, 'app-menu-edit');
    await user.click(screen.getByTestId('app-menu-edit-paste'));
    await waitFor(() => expect(screen.getByTestId('test-field-b')).toHaveValue('web-1'));
    expect(execCommand).toHaveBeenCalledWith('insertText', false, 'web-1');

    await openSubmenu(user, 'app-menu-edit');
    await user.click(screen.getByTestId('app-menu-edit-undo'));
    expect(execCommand).toHaveBeenCalledWith('undo', false, undefined);
    Reflect.deleteProperty(document, 'execCommand');
  });

  it('never copies or cuts a sensitive field, also while its value is shown', async () => {
    const execCommand = vi.fn(() => true);
    Object.defineProperty(document, 'execCommand', { configurable: true, value: execCommand });
    const { backend, user } = await renderUnlocked();
    const secret = screen.getByTestId('test-field-secret') as HTMLInputElement;
    await user.type(secret, 'revealed-secret-1');
    secret.setSelectionRange(0, secret.value.length);
    await openSubmenu(user, 'app-menu-edit');
    await user.click(screen.getByTestId('app-menu-edit-copy'));
    await openSubmenu(user, 'app-menu-edit');
    await user.click(screen.getByTestId('app-menu-edit-cut'));
    await act(async () => {
      await Promise.resolve();
    });
    expect(backend.control.clipboard()).toBeNull();
    expect(backend.control.calls()).not.toContain('clipboard_write_text');
    expect(execCommand).not.toHaveBeenCalledWith('delete', false, undefined);
    expect(secret).toHaveValue('revealed-secret-1');
    Reflect.deleteProperty(document, 'execCommand');
  });

  it('runs native menu bar actions (macOS) only while unlocked', async () => {
    const { backend } = await renderUnlocked({ os: 'macos' });
    expect(screen.queryByTestId('titlebar-menu-button')).not.toBeInTheDocument();
    await act(async () => {
      backend.control.emitMenuAction('app-menu-settings');
    });
    expect(await screen.findByTestId('settings-modal')).toBeInTheDocument();
  });

  it('closes on Escape and returns focus to the menu button', async () => {
    const { user } = await renderUnlocked();
    await openMenu(user);
    expect(screen.getByTestId('app-menu-vaults')).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('app-menu')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-menu-button')).toHaveFocus();
  });
});
