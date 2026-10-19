// Connect actions of the Hosts screen (UI 2.1, 2.2, 2.4; stage 2): quick
// connect from the search bar with the save offer, exact saved-host matches,
// double click, Enter, the context menu with Alt for a new window, the
// multi-selection, the Terminal button and OS icons on cards. The terminal
// workspace (S6) is replaced with spies.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetStores, vaultsStore, viewStore } from '../../app/store';
import { clearToasts, getToasts } from '../../app/toast';
import { setBackend, type SessionInfo, type SettingsValues, type Uuid } from '../../ipc';
import { activateTab, openLocalTerminal, openSession } from '../connect/workspaceBridge';
import { mergeSession, resetWorkspace } from '../terminal';
import { HostsScreen } from './HostsScreen';
import { createFakeBackend, FAKE_VAULT, FAKE_VAULT_ID, type FakeBackend } from './testing/fakeBackend';
import { getHostsViewState, resetHostsViewState } from './viewState';

vi.mock('../connect/workspaceBridge', () => ({
  openSession: vi.fn(() => Promise.resolve(null)),
  openLocalTerminal: vi.fn(() => Promise.resolve(null)),
  activateTab: vi.fn(),
  restoreLastSession: vi.fn(() => Promise.resolve()),
}));

const SETTINGS: SettingsValues = { 'view.hosts.layout': 'grid', 'view.hosts.sort': 'manual' };

let backend: FakeBackend;
let ids: Record<'web' | 'db' | 'lab', Uuid>;

beforeEach(() => {
  resetStores();
  resetWorkspace();
  resetHostsViewState();
  clearToasts();
  vi.mocked(openSession).mockReset();
  vi.mocked(openSession).mockResolvedValue(null);
  vi.mocked(openLocalTerminal).mockClear();
  vi.mocked(activateTab).mockClear();
  backend = createFakeBackend(SETTINGS);
  ids = {
    web: backend.addHost({
      address: 'web.example.com',
      label: 'Web',
      os: { family: 'linux', distro: 'ubuntu', source: 'banner', detected_at: '2026-10-01T00:00:00.000Z' },
    }),
    db: backend.addHost({ address: 'db.example.com', label: 'DB primary' }),
    lab: backend.addHost({ address: '192.0.2.60' }),
  };
  setBackend(backend);
  vaultsStore.set([FAKE_VAULT]);
});

afterEach(() => {
  cleanup();
  setBackend(null);
  resetStores();
  resetWorkspace();
  resetHostsViewState();
  clearToasts();
});

async function renderScreen() {
  const result = render(<HostsScreen vault={FAKE_VAULT} settings={SETTINGS} active selectRequest={null} />);
  await waitFor(() => expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-ready', 'true'));
  return result;
}

const hostCard = (id: Uuid) =>
  screen.getAllByTestId('host-card').find((card) => card.getAttribute('data-record-id') === id) as HTMLElement;
const hostTarget = (id: Uuid) => ({ kind: 'host', vault_id: FAKE_VAULT_ID, record_id: id });

describe('Hosts screen: search bar quick connect (UI 2.1)', () => {
  it('enables Connect only for an address or an exact saved-host match', async () => {
    const user = userEvent.setup();
    await renderScreen();
    const input = screen.getByTestId('hosts-search-input');
    const connect = screen.getByTestId('hosts-search-connect');
    expect(input).toHaveAttribute('placeholder', expect.stringContaining('ssh user@hostname'));
    expect(connect).toBeDisabled();
    await user.type(input, 'primary');
    expect(connect).toBeDisabled();
    await user.clear(input);
    await user.type(input, 'db primary');
    expect(connect).toBeEnabled();
    await user.clear(input);
    await user.type(input, 'admin@198.51.100.20:2222');
    expect(connect).toBeEnabled();
  });

  it('connects an exact match to the saved host', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.type(screen.getByTestId('hosts-search-input'), 'DB PRIMARY{Enter}');
    expect(openSession).toHaveBeenCalledWith(hostTarget(ids.db), { newWindow: false });
  });

  it('quick connects an address without saving it; Alt+click opens a new window', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.type(screen.getByTestId('hosts-search-input'), 'ssh ops@198.51.100.20 -p 2222{Enter}');
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'quick', address: '198.51.100.20', username: 'ops', port: 2222 }, { newWindow: false });
    await user.keyboard('{Alt>}');
    await user.click(screen.getByTestId('hosts-search-connect'));
    await user.keyboard('{/Alt}');
    expect(openSession).toHaveBeenLastCalledWith({ kind: 'quick', address: '198.51.100.20', username: 'ops', port: 2222 }, { newWindow: true });
    expect(backend.callsOf('record_save')).toEqual([]);
  });

  it('offers to save a quick-connect host once connected and opens its panel', async () => {
    const user = userEvent.setup();
    const sessionId = '01990000-0000-7000-8000-0000000000d1';
    vi.mocked(openSession).mockResolvedValue({ tabId: 't-1', paneId: 'p-1', sessionId });
    await renderScreen();
    await user.type(screen.getByTestId('hosts-search-input'), '198.51.100.21{Enter}');
    await waitFor(() => expect(openSession).toHaveBeenCalled());
    // The address was used: the search is cleared once the session opened.
    await waitFor(() => expect(screen.getByTestId('hosts-search-input')).toHaveValue(''));
    const base: SessionInfo = {
      session_id: sessionId,
      rev: 1,
      kind: 'ssh',
      window: 'w-1',
      state: 'connecting',
      title: '198.51.100.21',
      address: '198.51.100.21',
      port: 2022,
      username: 'asked-user',
      terminal_theme: { builtin: 'graphite' },
      backspace: 'default',
      record_history: true,
    };
    act(() => void mergeSession(base));
    expect(getToasts().map((toast) => toast.testId)).not.toContain('hosts-quick-connect-save-toast');
    act(() => void mergeSession({ ...base, rev: 2, state: 'connected' }));
    const toast = getToasts().find((item) => item.testId === 'hosts-quick-connect-save-toast');
    expect(toast?.action?.testId).toBe('hosts-quick-connect-save-toast-action');

    await act(async () => {
      toast?.action?.onAction();
    });
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(1));
    expect(backend.callsOf('record_save')[0]).toEqual({
      vault_id: FAKE_VAULT_ID,
      record_id: null,
      type: 'host',
      data: { address: '198.51.100.21', ssh: { port: 2022, username: 'asked-user' } },
    });
    const created = getHostsViewState().panel;
    expect(created?.type).toBe('host');
    expect(created?.record_id).not.toBeNull();
    expect(activateTab).toHaveBeenCalledWith('vaults');
    expect(viewStore.get().vaultsSection).toBe('hosts');
  });

  it('makes no save offer for a saved host or a session that never connects', async () => {
    const user = userEvent.setup();
    vi.mocked(openSession).mockResolvedValue({ tabId: 't-1', paneId: 'p-1', sessionId: 's-1' });
    await renderScreen();
    await user.type(screen.getByTestId('hosts-search-input'), 'Web{Enter}');
    await waitFor(() => expect(openSession).toHaveBeenCalled());
    act(
      () =>
        void mergeSession({
          session_id: 's-1',
          rev: 1,
          kind: 'ssh',
          window: 'w-1',
          state: 'connected',
          title: 'Web',
          terminal_theme: { builtin: 'graphite' },
          backspace: 'default',
          record_history: true,
        }),
    );
    expect(getToasts().map((toast) => toast.testId)).not.toContain('hosts-quick-connect-save-toast');
  });
});

describe('Hosts screen: card actions (UI 2.4)', () => {
  it('shows the detected OS icon, neutral until detected', async () => {
    await renderScreen();
    expect(hostCard(ids.web)).toHaveAttribute('data-os-family', 'linux');
    expect(hostCard(ids.web)).toHaveAttribute('data-os-distro', 'ubuntu');
    expect(within(hostCard(ids.web)).getByTestId('host-card-os-icon')).toHaveAttribute('data-os-family', 'linux');
    expect(hostCard(ids.db)).not.toHaveAttribute('data-os-family');
    expect(within(hostCard(ids.db)).getByTestId('host-card-os-icon')).not.toHaveAttribute('data-os-family');
  });

  it('connects on double click; Alt + double click opens a new window', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.dblClick(hostCard(ids.db));
    expect(openSession).toHaveBeenLastCalledWith(hostTarget(ids.db), { newWindow: false });
    expect(hostCard(ids.db)).toHaveAttribute('data-selected', 'true');
    await user.keyboard('{Alt>}');
    await user.dblClick(hostCard(ids.lab));
    await user.keyboard('{/Alt}');
    expect(openSession).toHaveBeenLastCalledWith(hostTarget(ids.lab), { newWindow: true });
  });

  it('connects the card of the first click when the grid moved between the clicks', async () => {
    await renderScreen();
    // The first click opens the details panel; the narrower grid puts another
    // card under the pointer for the second click.
    fireEvent.click(hostCard(ids.db), { detail: 1 });
    fireEvent.click(hostCard(ids.lab), { detail: 2 });
    fireEvent.doubleClick(hostCard(ids.lab), { detail: 2 });
    expect(openSession).toHaveBeenCalledTimes(1);
    expect(openSession).toHaveBeenLastCalledWith(hostTarget(ids.db), { newWindow: false });
    expect(hostCard(ids.db)).toHaveAttribute('data-selected', 'true');
    expect(hostCard(ids.lab)).toHaveAttribute('data-selected', 'false');

    // The second click can also land between the cards.
    fireEvent.click(hostCard(ids.web), { detail: 1 });
    fireEvent.doubleClick(hostCard(ids.web).parentElement as HTMLElement, { detail: 2 });
    expect(openSession).toHaveBeenLastCalledWith(hostTarget(ids.web), { newWindow: false });

    // A double click that starts elsewhere right after a card click is not taken.
    fireEvent.click(hostCard(ids.lab), { detail: 1 });
    const input = screen.getByTestId('hosts-search-input');
    fireEvent.mouseDown(input, { detail: 1 });
    fireEvent.click(input, { detail: 1 });
    fireEvent.mouseDown(input, { detail: 2 });
    fireEvent.click(input, { detail: 2 });
    fireEvent.doubleClick(input, { detail: 2 });
    expect(openSession).toHaveBeenCalledTimes(2);
  });

  it('connects every selected host with Enter, one tab each', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(hostCard(ids.web));
    await user.keyboard('{Control>}');
    await user.click(hostCard(ids.lab));
    await user.keyboard('{/Control}');
    hostCard(ids.lab).focus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(openSession).toHaveBeenCalledTimes(2));
    expect(vi.mocked(openSession).mock.calls.map((call) => call[0])).toEqual([hostTarget(ids.web), hostTarget(ids.lab)]);
  });

  it('offers Quick Connect and Connect > via SSH with the Alt hint; SFTP is hidden', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.db), { clientX: 10, clientY: 10 });
    expect(screen.getByTestId('host-menu-quick-connect')).toHaveTextContent('Enter');
    await user.click(screen.getByTestId('host-menu-connect'));
    const submenu = screen.getByTestId('host-menu-connect-menu');
    expect(within(submenu).getByTestId('host-menu-connect-ssh')).toBeInTheDocument();
    expect(within(submenu).getByTestId('host-menu-connect-alt-hint')).toHaveAttribute('aria-disabled', 'true');
    expect(screen.queryByTestId('host-menu-connect-sftp')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('host-menu-connect-ssh'));
    expect(openSession).toHaveBeenLastCalledWith(hostTarget(ids.db), { newWindow: false });

    fireEvent.contextMenu(hostCard(ids.lab), { clientX: 10, clientY: 10 });
    await user.keyboard('{Alt>}');
    await user.click(screen.getByTestId('host-menu-quick-connect'));
    await user.keyboard('{/Alt}');
    expect(openSession).toHaveBeenLastCalledWith(hostTarget(ids.lab), { newWindow: true });
  });

  it('connects every selected host from the context menu', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(hostCard(ids.db));
    await user.keyboard('{Control>}');
    await user.click(hostCard(ids.lab));
    await user.keyboard('{/Control}');
    fireEvent.contextMenu(hostCard(ids.lab), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('host-menu-quick-connect'));
    await waitFor(() => expect(openSession).toHaveBeenCalledTimes(2));
    expect(vi.mocked(openSession).mock.calls.map((call) => call[0])).toEqual([hostTarget(ids.db), hostTarget(ids.lab)]);
  });
});

describe('Hosts screen: Terminal button (UI 2.2)', () => {
  it('opens a local terminal in a new tab', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(screen.getByTestId('hosts-local-terminal-button'));
    expect(openLocalTerminal).toHaveBeenCalledTimes(1);
  });
});
