// Session tabs in the title bar (UI 1.1, 4.4, 4.6): OS icon and close button,
// the tab context menu (items, hidden later-stage items, duplicate, rename,
// splits, close), the neighbor rule, the sidebar toggle and the exit and
// window close confirmations (decision 17).

import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderUnlocked, resetApp } from '../../test/app';
import { closeActiveTab, requestAppExit } from './api';
import { engineFor, openLocal, withVault } from './testing/helpers';
import { leavesOf, workspaceStore } from './workspace';

vi.mock('../hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
});

const tabs = () => screen.getAllByTestId('titlebar-session-tab');

describe('session tabs', () => {
  it('shows the OS icon on inactive tabs, the close button on the active one, and the session type icon', async () => {
    await withVault(renderUnlocked());
    await openLocal();
    await openLocal();
    const [first, second] = tabs();
    expect(second).toHaveAttribute('aria-selected', 'true');
    expect(within(second as HTMLElement).queryByTestId('titlebar-session-tab-os-icon')).not.toBeInTheDocument();
    expect(within(second as HTMLElement).getByTestId('titlebar-session-tab-close')).toHaveAttribute('data-visible', 'true');
    expect(within(first as HTMLElement).getByTestId('titlebar-session-tab-os-icon')).toBeInTheDocument();
    expect(within(first as HTMLElement).getByTestId('titlebar-session-tab-kind')).toHaveAttribute('data-kind', 'terminal');
    expect(within(first as HTMLElement).getByTestId('titlebar-session-tab-title')).toHaveTextContent('PowerShell');
    expect(first).toHaveAttribute('data-kind', 'session');
    expect(first).toHaveAttribute('data-state', 'connected');
  });

  it('applies the neighbor rule to every way of closing the active tab', async () => {
    const { user } = await withVault(renderUnlocked());
    await openLocal();
    await openLocal();
    await openLocal();
    const ids = tabs().map((tab) => tab.getAttribute('data-tab-id'));
    await user.click(tabs()[1] as HTMLElement);
    await user.click(within(tabs()[1] as HTMLElement).getByTestId('titlebar-session-tab-close'));
    expect(workspaceStore.get().active).toBe(ids[0]);
    act(() => closeActiveTab());
    expect(workspaceStore.get().active).toBe(ids[2]);
    fireEvent(tabs()[0] as HTMLElement, new MouseEvent('auxclick', { bubbles: true, button: 1 }));
    expect(workspaceStore.get().active).toBe('vaults');
    expect(screen.queryByTestId('titlebar-session-tab')).not.toBeInTheDocument();
  });

  it('has the tab menu of UI 4.4 without the later-stage items', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    await openLocal();
    fireEvent.contextMenu(tabs()[0] as HTMLElement, { clientX: 40, clientY: 20 });
    const menu = await screen.findByTestId('session-tab-menu');
    const items = within(menu)
      .getAllByRole('menuitem')
      .map((item) => item.getAttribute('data-testid'));
    expect(items).toEqual([
      'session-tab-menu-duplicate',
      'session-tab-menu-duplicate-window',
      'session-tab-menu-rename',
      'session-tab-menu-split-horizontal',
      'session-tab-menu-split-vertical',
      'session-tab-menu-close',
    ]);
    expect(screen.queryByTestId('session-tab-menu-multiplayer')).not.toBeInTheDocument();
    expect(screen.queryByTestId('session-tab-menu-broadcast')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('session-tab-menu-duplicate'));
    await waitFor(() => expect(tabs()).toHaveLength(2));
    expect(backend.control.sessions.list()).toHaveLength(2);

    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    await user.click(await screen.findByTestId('session-tab-menu-duplicate-window'));
    await waitFor(() => expect(backend.control.windowIntents()).toEqual([{ kind: 'open_local', shell_id: 'powershell' }]));
    expect(tabs()).toHaveLength(2);
  });

  it('renames a tab inline: Enter saves, Escape cancels, an empty name restores the session title', async () => {
    const { user } = await withVault(renderUnlocked());
    await openLocal();
    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    await user.click(await screen.findByTestId('session-tab-menu-rename'));
    const input = await screen.findByTestId('titlebar-session-tab-rename-input');
    await waitFor(() => expect(input).toHaveFocus());
    await user.clear(input);
    await user.type(input, 'build box{Enter}');
    expect(screen.getByTestId('titlebar-session-tab-title')).toHaveTextContent('build box');

    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    await user.click(await screen.findByTestId('session-tab-menu-rename'));
    await user.type(await screen.findByTestId('titlebar-session-tab-rename-input'), ' more{Escape}');
    expect(screen.getByTestId('titlebar-session-tab-title')).toHaveTextContent('build box');

    fireEvent.keyDown(tabs()[0] as HTMLElement, { key: 'F2' });
    const again = await screen.findByTestId('titlebar-session-tab-rename-input');
    await user.clear(again);
    await user.type(again, '{Enter}');
    expect(screen.getByTestId('titlebar-session-tab-title')).toHaveTextContent('PowerShell');
  });

  it('does not rename a new-tab page (it is "New Tab" until a host is chosen)', async () => {
    const { user } = await withVault(renderUnlocked());
    await user.click(screen.getByTestId('titlebar-new-tab'));
    await screen.findByTestId('newtab-page');
    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    expect(await screen.findByTestId('session-tab-menu-rename')).toHaveAttribute('aria-disabled', 'true');
    await user.keyboard('{Escape}');
    fireEvent.keyDown(tabs()[0] as HTMLElement, { key: 'F2' });
    expect(screen.queryByTestId('titlebar-session-tab-rename-input')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-session-tab-title')).toHaveTextContent('New Tab');
  });

  it('splits from the tab menu and closes the tab with its sessions', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    const { tabId, sessionId } = await openLocal();
    await engineFor(sessionId);
    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    await user.click(await screen.findByTestId('session-tab-menu-split-horizontal'));
    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    await user.click(await screen.findByTestId('session-tab-menu-split-vertical'));
    const root = workspaceStore.get().tabs.find((tab) => tab.id === tabId)?.root ?? null;
    expect(leavesOf(root)).toHaveLength(3);
    expect(root).toMatchObject({ type: 'split', direction: 'column' });
    expect(screen.getAllByTestId('terminal-pane')).toHaveLength(3);

    fireEvent.contextMenu(tabs()[0] as HTMLElement);
    await user.click(await screen.findByTestId('session-tab-menu-close'));
    expect(screen.queryByTestId('titlebar-session-tab')).not.toBeInTheDocument();
    await waitFor(() => expect(backend.control.sessions.list()).toEqual([]));
  });

  it('shows the terminal sidebar toggle only while a session tab is active', async () => {
    const { user } = await withVault(renderUnlocked());
    await user.click(screen.getByTestId('titlebar-new-tab'));
    expect(screen.queryByTestId('titlebar-terminal-sidebar-toggle')).not.toBeInTheDocument();
    expect(await screen.findByTestId('newtab-page')).toBeInTheDocument();
    await openLocal();
    const toggle = screen.getByTestId('titlebar-terminal-sidebar-toggle');
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('terminal-sidebar')).toHaveAttribute('data-tab', 'tools');
    expect(screen.queryByTestId('terminal-sidebar-tab-snippets')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('titlebar-tab-vaults'));
    expect(screen.queryByTestId('titlebar-terminal-sidebar-toggle')).not.toBeInTheDocument();
  });
});

describe('close confirmation', () => {
  it('asks before closing a window with active sessions', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    await openLocal();
    await user.click(screen.getByTestId('titlebar-window-close'));
    const dialog = await screen.findByTestId('window-close-confirm-dialog');
    expect(dialog).toHaveAttribute('data-count', '1');
    await user.click(screen.getByTestId('window-close-confirm-dialog-cancel'));
    expect(backend.control.windowState().closed).toBe(false);

    act(() => backend.control.emitCloseRequested());
    await user.click(await screen.findByTestId('window-close-confirm-dialog-confirm'));
    await waitFor(() => expect(backend.control.windowState().closed).toBe(true));
  });

  it('asks before exiting with active sessions and writes the last session first', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    await openLocal();
    await act(async () => {
      await requestAppExit();
    });
    const dialog = await screen.findByTestId('exit-confirm-dialog');
    expect(dialog).toHaveAttribute('data-count', '1');
    expect(backend.control.exitRequested()).toBe(false);
    expect((await backend.invoke('settings_get', undefined))['session.last']).toMatchObject({
      tabs: [{ root: { session: { kind: 'local' } } }],
    });
    await user.click(screen.getByTestId('exit-confirm-dialog-confirm'));
    await waitFor(() => expect(backend.control.exitRequested()).toBe(true));
  });

  it('exits at once without active sessions', async () => {
    const { backend } = await withVault(renderUnlocked());
    await act(async () => {
      await requestAppExit();
    });
    expect(backend.control.exitRequested()).toBe(true);
    expect(screen.queryByTestId('exit-confirm-dialog')).not.toBeInTheDocument();
  });
});
