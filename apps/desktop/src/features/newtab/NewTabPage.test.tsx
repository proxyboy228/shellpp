// New tab page, host picker and the Ctrl+K palette (UI 6; stage 2 plan
// decision 13) against the mock backend. Opening sessions belongs to the
// terminal workspace (S6), so its entry points are replaced with spies.

import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { lockStore, resetStores, settingsStore, vaultsStore } from '../../app/store';
import { setBackend, type HostView, type SessionInfo, type VaultView } from '../../ipc';
import { createMockBackend, type MockBackend } from '../../ipc/mock';
import { activateTab, openSession, restoreLastSession } from '../connect/workspaceBridge';
import { applySnapshotSessions, resetWorkspace, sessionsStore, workspaceStore } from '../terminal';
import { NewTabPage } from './NewTabPage';
import { closeSearchPalette, openSearchPalette, SearchPalette } from './SearchPalette';

vi.mock('../connect/workspaceBridge', () => ({
  openSession: vi.fn(() => Promise.resolve(null)),
  openLocalTerminal: vi.fn(() => Promise.resolve(null)),
  activateTab: vi.fn(),
  restoreLastSession: vi.fn(() => Promise.resolve()),
}));

let backend: MockBackend;
let vault: VaultView;
let hosts: Record<'db' | 'web' | 'lab', HostView>;

async function addHost(data: HostView['data']): Promise<HostView> {
  const view = await backend.invoke('record_save', { vault_id: vault.vault_id, record_id: null, type: 'host', data });
  return view as HostView;
}

beforeEach(async () => {
  resetStores();
  resetWorkspace();
  vi.mocked(openSession).mockClear();
  vi.mocked(activateTab).mockClear();
  vi.mocked(restoreLastSession).mockClear();
  backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
  setBackend(backend);
  const snapshot = await backend.invoke('state_snapshot', undefined);
  vault = snapshot.vaults[0] as VaultView;
  lockStore.set(snapshot.lock);
  vaultsStore.set(snapshot.vaults);
  settingsStore.set(snapshot.settings);
  const group = await backend.invoke('record_save', { vault_id: vault.vault_id, record_id: null, type: 'group', data: { name: 'prod' } });
  hosts = {
    db: await addHost({ address: 'db.example.com', label: 'Database', group_id: group.record_id }),
    web: await addHost({ address: 'web.example.com', label: 'Web front', tags: ['frontend'] }),
    lab: await addHost({ address: '192.0.2.40', ssh: { username: 'labuser' } }),
  };
});

afterEach(() => {
  setBackend(null);
  resetStores();
  resetWorkspace();
  closeSearchPalette();
});

function addConnections() {
  const control = backend.control.connect;
  control.addConnection({
    host: { vault_id: vault.vault_id, record_id: hosts.db.record_id },
    title: 'Database',
    address: 'db.example.com',
    port: 22,
    last_at: '2026-10-03T10:00:00.000Z',
  });
  control.addConnection({ title: '198.51.100.9', address: '198.51.100.9', port: 2200, username: 'ops', last_at: '2026-10-02T10:00:00.000Z' });
}

const SESSION: SessionInfo = {
  session_id: '01990000-0000-7000-8000-0000000000e1',
  rev: 1,
  kind: 'ssh',
  window: 'w-1',
  state: 'connected',
  title: 'Web front',
  address: 'web.example.com',
  port: 22,
  terminal_theme: { builtin: 'graphite' },
  backspace: 'default',
  record_history: true,
};

/** A window with one session tab (`t-1`) and the new tab (`t-2`). */
function openTabs() {
  sessionsStore.set(new Map([[SESSION.session_id, SESSION]]));
  workspaceStore.set({
    tabs: [
      { id: 't-1', kind: 'session', customTitle: null, root: { type: 'leaf', paneId: 'p-1', sessionId: SESSION.session_id }, activePaneId: 'p-1' },
      { id: 't-2', kind: 'newtab', customTitle: null, root: null, activePaneId: null },
    ],
    active: 't-2',
    sidebarOpen: false,
    sidebarTab: 'tools',
  });
}

async function renderPage() {
  render(<NewTabPage tabId="t-2" active />);
  await waitFor(() =>
    expect(screen.queryAllByTestId('newtab-recent-item').length + screen.queryAllByTestId('newtab-recent-empty').length).toBeGreaterThan(0),
  );
}

const recentRows = () => screen.queryAllByTestId('newtab-recent-item');

describe('NewTabPage: Recent connections (UI 6)', () => {
  it('focuses the search with the Ctrl+K hint and shows an empty Recent block', async () => {
    await renderPage();
    expect(screen.getByTestId('newtab-page')).toHaveAttribute('data-tab-id', 't-2');
    expect(screen.getByTestId('newtab-search-input')).toHaveFocus();
    expect(screen.getByTestId('newtab-search-hint')).toHaveTextContent('Ctrl+K');
    expect(screen.getByTestId('newtab-recent-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('newtab-create-workspace')).not.toBeInTheDocument();
    expect(screen.queryByTestId('newtab-workspaces')).not.toBeInTheDocument();
  });

  it('lists recent connections with OS icon, title and "vault / group" path', async () => {
    addConnections();
    await renderPage();
    const [db, quick] = recentRows();
    expect(db).toHaveAttribute('data-host-id', hosts.db.record_id);
    expect(within(db as HTMLElement).getByTestId('newtab-recent-item-title')).toHaveTextContent('Database');
    expect(within(db as HTMLElement).getByTestId('newtab-recent-item-path')).toHaveTextContent('Personal / prod');
    // The mock never detects an OS: the neutral icon (the OS family comes from the core's recent rows).
    expect(within(db as HTMLElement).getByTestId('newtab-recent-item-os-icon')).not.toHaveAttribute('data-os-family');
    expect(quick).toHaveAttribute('data-address', '198.51.100.9');
    expect(quick).not.toHaveAttribute('data-host-id');
    expect(within(quick as HTMLElement).getByTestId('newtab-recent-item-path')).toHaveTextContent(/^Personal$/);
  });

  it('connects in this tab on click; Alt opens a new window', async () => {
    const user = userEvent.setup();
    addConnections();
    await renderPage();
    await user.click(recentRows()[0] as HTMLElement);
    expect(openSession).toHaveBeenLastCalledWith(
      { kind: 'host', vault_id: vault.vault_id, record_id: hosts.db.record_id },
      { inTab: 't-2' },
    );
    await user.keyboard('{Alt>}');
    await user.click(recentRows()[1] as HTMLElement);
    await user.keyboard('{/Alt}');
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'quick', address: '198.51.100.9', port: 2200, username: 'ops' }, { newWindow: true });
  });

  it('moves the cursor with the arrows and connects with Enter', async () => {
    const user = userEvent.setup();
    addConnections();
    await renderPage();
    expect(recentRows()[0]).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{ArrowDown}');
    expect(recentRows()[1]).toHaveAttribute('aria-selected', 'true');
    expect(recentRows()[0]).toHaveAttribute('aria-selected', 'false');
    await user.keyboard('{ArrowDown}');
    expect(recentRows()[0]).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{ArrowUp}{Enter}');
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'quick', address: '198.51.100.9', port: 2200, username: 'ops' }, { inTab: 't-2' });
  });

  it('refreshes Recent connections when the connection log changes', async () => {
    await renderPage();
    expect(recentRows()).toHaveLength(0);
    act(() => addConnections());
    await waitFor(() => expect(recentRows()).toHaveLength(2));
  });

  it('drops the lists while the window is locked', async () => {
    addConnections();
    await renderPage();
    expect(recentRows()).toHaveLength(2);
    act(() => {
      lockStore.set({ state: 'locked', remember_device_available: true, remember_device_enabled: false });
      vaultsStore.set([]);
    });
    expect(recentRows()).toHaveLength(0);
  });
});

describe('NewTabPage: search results', () => {
  it('lists open tabs first, then hosts by label, address, username and tags', async () => {
    const user = userEvent.setup();
    openTabs();
    await renderPage();
    await user.type(screen.getByTestId('newtab-search-input'), 'web');
    const results = screen.getByTestId('newtab-results');
    const order = within(results)
      .getAllByRole('option')
      .map((item) => item.getAttribute('data-testid'));
    expect(order).toEqual(['newtab-result-tab', 'newtab-result-host']);
    expect(screen.getByTestId('newtab-result-tab')).toHaveAttribute('data-tab-id', 't-1');
    expect(screen.getByTestId('newtab-result-host')).toHaveAttribute('data-host-id', hosts.web.record_id);

    await user.clear(screen.getByTestId('newtab-search-input'));
    await user.type(screen.getByTestId('newtab-search-input'), 'labuser');
    expect(screen.getAllByTestId('newtab-result-host').map((item) => item.getAttribute('data-host-id'))).toEqual([hosts.lab.record_id]);
    expect(screen.queryByTestId('newtab-result-tab')).not.toBeInTheDocument();

    await user.clear(screen.getByTestId('newtab-search-input'));
    await user.type(screen.getByTestId('newtab-search-input'), 'FRONTEND');
    expect(screen.getAllByTestId('newtab-result-host').map((item) => item.getAttribute('data-host-id'))).toEqual([hosts.web.record_id]);
  });

  it('activates a chosen tab and connects a chosen host in this tab', async () => {
    const user = userEvent.setup();
    openTabs();
    await renderPage();
    await user.type(screen.getByTestId('newtab-search-input'), 'web');
    await user.keyboard('{Enter}');
    expect(activateTab).toHaveBeenCalledWith('t-1');
    expect(openSession).not.toHaveBeenCalled();
    await user.keyboard('{ArrowDown}{Enter}');
    expect(openSession).toHaveBeenLastCalledWith(
      { kind: 'host', vault_id: vault.vault_id, record_id: hosts.web.record_id },
      { inTab: 't-2' },
    );
  });

  it('shows "no results" and clears the search with Escape', async () => {
    const user = userEvent.setup();
    await renderPage();
    await user.type(screen.getByTestId('newtab-search-input'), 'zzz-nothing');
    expect(screen.getByTestId('newtab-no-results')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.getByTestId('newtab-search-input')).toHaveValue('');
    expect(screen.getByTestId('newtab-recent')).toBeInTheDocument();
  });
});

describe('NewTabPage: Restore', () => {
  it('is disabled without a last session and restores it when there is one', async () => {
    const user = userEvent.setup();
    await renderPage();
    expect(screen.getByTestId('newtab-restore')).toBeDisabled();
    act(() =>
      applySnapshotSessions([], undefined, {
        settings: {
          ...settingsStore.get(),
          'session.last': { saved_at: '2026-10-18T10:00:00.000Z', tabs: [{ root: { session: { kind: 'local' } } }] },
        },
        launchedAt: '2026-10-19T09:00:00.000Z',
        unlocked: true,
      }),
    );
    expect(screen.getByTestId('newtab-restore')).toBeEnabled();
    await user.click(screen.getByTestId('newtab-restore'));
    expect(restoreLastSession).toHaveBeenCalledTimes(1);
  });

  it('stays disabled when the stored tabs were written during this launch', async () => {
    await renderPage();
    act(() =>
      applySnapshotSessions([], undefined, {
        settings: {
          ...settingsStore.get(),
          'session.last': { saved_at: '2026-10-19T09:00:05.000Z', tabs: [{ root: { session: { kind: 'local' } } }] },
        },
        launchedAt: '2026-10-19T09:00:00.000Z',
        unlocked: true,
      }),
    );
    expect(screen.getByTestId('newtab-restore')).toBeDisabled();
  });
});

describe('SearchPalette (Ctrl+K)', () => {
  it('opens with the search focused and closes with Escape', async () => {
    const user = userEvent.setup();
    render(<SearchPalette />);
    expect(screen.queryByTestId('search-palette')).not.toBeInTheDocument();
    act(() => openSearchPalette());
    expect(await screen.findByTestId('search-palette')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('search-palette-search-input')).toHaveFocus());
    expect(screen.getByTestId('search-palette-search-hint')).toHaveTextContent('Ctrl+K');
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('search-palette')).not.toBeInTheDocument();
  });

  it('connects a host in a new tab and activates a tab, closing each time', async () => {
    const user = userEvent.setup();
    openTabs();
    render(<SearchPalette />);
    act(() => openSearchPalette());
    await user.type(await screen.findByTestId('search-palette-search-input'), 'Database');
    await waitFor(() => expect(screen.getByTestId('search-palette-result-host')).toBeInTheDocument());
    await user.click(screen.getByTestId('search-palette-result-host'));
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'host', vault_id: vault.vault_id, record_id: hosts.db.record_id }, {});
    expect(screen.queryByTestId('search-palette')).not.toBeInTheDocument();

    act(() => openSearchPalette());
    await user.type(await screen.findByTestId('search-palette-search-input'), 'web front');
    await waitFor(() => expect(screen.getByTestId('search-palette-result-tab')).toBeInTheDocument());
    await user.click(screen.getByTestId('search-palette-result-tab'));
    expect(activateTab).toHaveBeenCalledWith('t-1');
    expect(screen.queryByTestId('search-palette')).not.toBeInTheDocument();
  });

  it('does not come back after its window content unmounts (lock)', async () => {
    const { unmount } = render(<SearchPalette />);
    act(() => openSearchPalette());
    await screen.findByTestId('search-palette');
    unmount();
    render(<SearchPalette />);
    expect(screen.queryByTestId('search-palette')).not.toBeInTheDocument();
  });
});
