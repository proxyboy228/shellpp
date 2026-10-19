// Host picker details (UI 6, 4.7): OS icons of recent rows, the split-pane
// variant without tabs, Escape, and the picker model.

import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { lockStore, resetStores, vaultsStore } from '../../app/store';
import { setBackend, type HostView, type RecentConnection, type SessionInfo } from '../../ipc';
import { createRecordingBackend, type RecordingBackend } from '../connect/testing/recordingBackend';
import { resetWorkspace, sessionsStore, summarizeTabs, workspaceStore } from '../terminal';
import { HostPicker } from './HostPicker';
import { hostMatches, pickerItems, recentTarget } from './pickerModel';

const VAULT_ID = '01990000-0000-7000-8000-00000000aaaa';

function host(id: string, data: HostView['data'], inherited: HostView['inherited'] = {}): HostView {
  return {
    type: 'host',
    vault_id: VAULT_ID,
    record_id: id,
    revision: 1,
    created_at: '2026-10-01T00:00:00.000Z',
    modified_at: '2026-10-01T00:00:00.000Z',
    inherited,
    path: [],
    data,
    secrets_set: [],
  };
}

const HOSTS = [
  host('01990000-0000-7000-8000-0000000000f1', { address: 'db.example.com', label: 'Database', ssh: { port: 2200 } }),
  host('01990000-0000-7000-8000-0000000000f2', { address: '192.0.2.50' }, { username: { value: 'deploy', from_group_id: VAULT_ID } }),
];

const RECENT: RecentConnection[] = [
  {
    host: { vault_id: VAULT_ID, record_id: HOSTS[0]?.record_id ?? '' },
    title: 'Database',
    address: 'db.example.com',
    port: 2200,
    os: { family: 'linux', distro: 'ubuntu', source: 'banner', detected_at: '2026-10-01T00:00:00.000Z' },
    path: ['Personal', 'prod', 'eu'],
    last_at: '2026-10-03T10:00:00.000Z',
  },
];

let backend: RecordingBackend;

beforeEach(() => {
  resetStores();
  resetWorkspace();
  backend = createRecordingBackend({
    recent_connections: () => RECENT,
    records_query: () => HOSTS,
  });
  setBackend(backend);
  lockStore.set({ state: 'unlocked', remember_device_available: true, remember_device_enabled: false });
  vaultsStore.set([{ vault_id: VAULT_ID, kind: 'personal', status: 'active', name: 'Personal', color: 'graphite' }]);
});

afterEach(() => {
  setBackend(null);
  resetStores();
  resetWorkspace();
});

const SESSION: SessionInfo = {
  session_id: '01990000-0000-7000-8000-0000000000e2',
  rev: 1,
  kind: 'ssh',
  window: 'w-1',
  state: 'connected',
  title: 'Database',
  address: 'db.example.com',
  terminal_theme: { builtin: 'graphite' },
  backspace: 'default',
  record_history: true,
};

describe('HostPicker', () => {
  it('shows the OS icon and the path of a recent row', async () => {
    render(<HostPicker testIdPrefix="pane-host-picker" onPickHost={vi.fn()} />);
    const row = await screen.findByTestId('pane-host-picker-recent-item');
    expect(within(row).getByTestId('pane-host-picker-recent-item-os-icon')).toHaveAttribute('data-os-family', 'linux');
    expect(within(row).getByTestId('pane-host-picker-recent-item-os-icon')).toHaveAttribute('data-os-distro', 'ubuntu');
    expect(within(row).getByTestId('pane-host-picker-recent-item-path')).toHaveTextContent('Personal / prod / eu');
    expect(screen.getByTestId('pane-host-picker-search-input')).toHaveFocus();
  });

  it('lists no tabs in a split pane and shows host addresses with user and port', async () => {
    const user = userEvent.setup();
    sessionsStore.set(new Map([[SESSION.session_id, SESSION]]));
    workspaceStore.set({
      tabs: [{ id: 't-1', kind: 'session', customTitle: null, root: { type: 'leaf', paneId: 'p-1', sessionId: SESSION.session_id }, activePaneId: 'p-1' }],
      active: 't-1',
      sidebarOpen: false,
      sidebarTab: 'tools',
    });
    render(<HostPicker testIdPrefix="pane-host-picker" includeTabs={false} onPickHost={vi.fn()} />);
    await screen.findByTestId('pane-host-picker-recent-item');
    await user.type(screen.getByTestId('pane-host-picker-search-input'), '.');
    await waitFor(() => expect(screen.getAllByTestId('pane-host-picker-result-host')).toHaveLength(2));
    expect(screen.queryByTestId('pane-host-picker-result-tab')).not.toBeInTheDocument();
    // Sorted by title: the address-only host first.
    const [lab, db] = screen.getAllByTestId('pane-host-picker-result-host');
    expect(db).toHaveTextContent('db.example.com:2200');
    expect(lab).toHaveTextContent('deploy@192.0.2.50');
  });

  it('calls onCancel on Escape (the palette closes)', async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    render(<HostPicker testIdPrefix="search-palette" includeTabs onPickHost={vi.fn()} onCancel={onCancel} />);
    await screen.findByTestId('search-palette-recent-item');
    await user.type(screen.getByTestId('search-palette-search-input'), 'x{Escape}');
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('reloads Recent connections when it is shown again', async () => {
    const { rerender } = render(<HostPicker testIdPrefix="newtab" active onPickHost={vi.fn()} />);
    await screen.findByTestId('newtab-recent-item');
    const before = backend.callsOf('recent_connections').length;
    rerender(<HostPicker testIdPrefix="newtab" active={false} onPickHost={vi.fn()} />);
    rerender(<HostPicker testIdPrefix="newtab" active onPickHost={vi.fn()} />);
    await waitFor(() => expect(backend.callsOf('recent_connections').length).toBe(before + 1));
  });
});

describe('picker model', () => {
  it('targets the saved host of a recent row, else the address it used', () => {
    expect(recentTarget(RECENT[0] as RecentConnection)).toEqual({ kind: 'host', vault_id: VAULT_ID, record_id: HOSTS[0]?.record_id });
    expect(recentTarget({ title: 'x', address: '198.51.100.2', port: 22, path: ['Personal'], last_at: '2026-10-01T00:00:00.000Z' })).toEqual({
      kind: 'quick',
      address: '198.51.100.2',
    });
  });

  it('matches hosts by label, address, inherited username and tags', () => {
    const tagged = host('h-t', { address: 'x.example.com', tags: ['Blue'] });
    expect(hostMatches(HOSTS[0] as HostView, 'DATA')).toBe(true);
    expect(hostMatches(HOSTS[1] as HostView, 'deploy')).toBe(true);
    expect(hostMatches(tagged, 'blue')).toBe(true);
    expect(hostMatches(tagged, 'green')).toBe(false);
  });

  it('puts open session tabs first and skips new-tab pages', () => {
    const tabs = summarizeTabs(
      {
        tabs: [
          { id: 't-1', kind: 'session', customTitle: null, root: { type: 'leaf', paneId: 'p-1', sessionId: SESSION.session_id }, activePaneId: 'p-1' },
          { id: 't-2', kind: 'newtab', customTitle: null, root: null, activePaneId: null },
        ],
        active: 't-2',
        sidebarOpen: false,
        sidebarTab: 'tools',
      },
      new Map([[SESSION.session_id, SESSION]]),
    );
    const items = pickerItems({ query: 'data', includeTabs: true, recent: RECENT, tabs, hosts: HOSTS });
    expect(items.map((item) => item.kind)).toEqual(['tab', 'host']);
    expect(pickerItems({ query: '', includeTabs: true, recent: RECENT, tabs, hosts: HOSTS }).map((item) => item.kind)).toEqual(['recent']);
  });
});
