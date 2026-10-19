// Title bar (UI 1.1): visible elements, hidden later-stage elements, window
// buttons, dragging and the macOS layout.

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { appInfoStore } from '../store';
import { renderUnlocked, resetApp, setupBackend } from '../../test/app';
import { TitleBar } from './TitleBar';

vi.mock('../../features/hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
});

const HIDDEN = [
  'titlebar-vault-switcher',
  'titlebar-tab-sftp',
  'titlebar-session-tab',
  'titlebar-new-tab',
  'titlebar-update-button',
  'titlebar-terminal-sidebar-toggle',
];

describe('TitleBar', () => {
  it('shows the menu button, the selected Vaults tab, notifications and window buttons', async () => {
    await renderUnlocked();
    expect(screen.getByTestId('titlebar')).toBeInTheDocument();
    expect(screen.getByTestId('titlebar-menu-button')).toBeInTheDocument();
    const tab = screen.getByTestId('titlebar-tab-vaults');
    expect(tab).toHaveAttribute('role', 'tab');
    expect(tab).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('titlebar-notifications-button')).toBeInTheDocument();
    for (const id of ['titlebar-window-minimize', 'titlebar-window-maximize', 'titlebar-window-close']) {
      expect(screen.getByTestId(id)).toBeInTheDocument();
    }
    for (const id of HIDDEN) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
  });

  it('drives the window: minimize, maximize with data-maximized, close', async () => {
    const { backend, user } = await renderUnlocked();
    const maximize = screen.getByTestId('titlebar-window-maximize');
    expect(maximize).toHaveAttribute('data-maximized', 'false');
    await user.click(maximize);
    await waitFor(() => expect(maximize).toHaveAttribute('data-maximized', 'true'));
    await user.click(maximize);
    await waitFor(() => expect(maximize).toHaveAttribute('data-maximized', 'false'));
    await user.click(screen.getByTestId('titlebar-window-minimize'));
    expect(backend.control.windowState().minimized).toBe(true);
    await user.click(screen.getByTestId('titlebar-window-close'));
    expect(backend.control.windowState().closed).toBe(true);
  });

  it('drags from the empty bar area and toggles maximize on a double click', async () => {
    const backend = setupBackend({ initialState: 'unlocked' });
    render(<TitleBar variant="full" />);
    const bar = screen.getByTestId('titlebar');
    fireEvent.mouseDown(bar, { button: 0, detail: 1 });
    expect(backend.control.windowState().dragging).toBe(1);
    fireEvent.mouseDown(screen.getByTestId('titlebar-tab-vaults'), { button: 0, detail: 1 });
    expect(backend.control.windowState().dragging).toBe(1);
    await act(async () => {
      fireEvent.mouseDown(bar, { button: 0, detail: 2 });
    });
    expect(backend.control.windowState().maximized).toBe(true);
    fireEvent.mouseDown(bar, { button: 2, detail: 1 });
    expect(backend.control.windowState().dragging).toBe(1);
  });

  it('shows only the drag area and window buttons on the lock screens', () => {
    setupBackend();
    render(<TitleBar variant="minimal" />);
    expect(screen.queryByTestId('titlebar-menu-button')).not.toBeInTheDocument();
    expect(screen.queryByTestId('titlebar-tab-vaults')).not.toBeInTheDocument();
    expect(screen.queryByTestId('titlebar-notifications-button')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-window-close')).toBeInTheDocument();
  });

  it('leaves the window buttons and the menu button to macOS', () => {
    setupBackend({ os: 'macos' });
    act(() => {
      appInfoStore.set({ name: 'Shell++', version: '0.0.0', os: 'macos', arch: 'aarch64', debug: true, e2e: false });
    });
    render(<TitleBar variant="full" />);
    expect(screen.getByTestId('titlebar')).toHaveAttribute('data-mac', 'true');
    expect(screen.queryByTestId('titlebar-menu-button')).not.toBeInTheDocument();
    expect(screen.queryByTestId('titlebar-window-close')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-tab-vaults')).toBeInTheDocument();
  });
});
