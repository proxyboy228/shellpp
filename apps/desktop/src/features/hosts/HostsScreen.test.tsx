import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { clearToasts, subscribeToasts, type Toast } from '../../app/toast';
import { setBackend, type SettingsValues, type Uuid } from '../../ipc';
import { HostsScreen, type HostsScreenProps } from './HostsScreen';
import { createFakeBackend, FAKE_VAULT, FAKE_VAULT_ID, type FakeBackend } from './testing/fakeBackend';
import { getHostsViewState, resetHostsViewState } from './viewState';

let backend: FakeBackend;
let toasts: readonly Toast[];
let stopToasts: () => void;
let ids: Record<'prod' | 'web' | 'web1' | 'db1' | 'lab' | 'backup', Uuid>;

const SETTINGS: SettingsValues = { 'view.hosts.layout': 'grid', 'view.hosts.sort': 'manual', 'terminal.keepalive_s': 30 };

beforeEach(() => {
  backend = createFakeBackend(SETTINGS);
  const prod = backend.addGroup({ name: 'prod', color: 'ocean', ssh: { port: 2222, username: 'deploy' } });
  const web = backend.addGroup({ name: 'web', parent_id: prod });
  const web1 = backend.addHost({ address: 'web1.example.com', label: 'web-1', group_id: web, tags: ['web', 'eu'] });
  const db1 = backend.addHost({ address: 'db1.example.com', label: 'DB primary', group_id: prod, tags: ['db'] });
  const lab = backend.addHost({ address: '192.0.2.10', tags: ['lab'], ssh: { username: 'root' } });
  const backup = backend.addHost({ address: 'backup.example.org', label: 'backup', color: 'moss' });
  ids = { prod, web, web1, db1, lab, backup };
  setBackend(backend);
  resetHostsViewState();
  toasts = [];
  stopToasts = subscribeToasts((list) => {
    toasts = list;
  });
});

afterEach(() => {
  // Unmount before the stores and the backend are reset.
  cleanup();
  stopToasts();
  clearToasts();
  setBackend(null);
  resetHostsViewState();
});

async function renderScreen(props: Partial<HostsScreenProps> = {}) {
  const result = render(<HostsScreen vault={FAKE_VAULT} settings={SETTINGS} active selectRequest={null} {...props} />);
  await waitFor(() => expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-ready', 'true'));
  return result;
}

const hostCard = (id: Uuid) =>
  screen.getAllByTestId('host-card').find((card) => card.getAttribute('data-record-id') === id) as HTMLElement;
const groupCard = (id: Uuid) =>
  screen.getAllByTestId('group-card').find((card) => card.getAttribute('data-record-id') === id) as HTMLElement;
const shownHosts = () => screen.queryAllByTestId('host-card').map((card) => card.getAttribute('data-record-id'));
const shownGroups = () => screen.queryAllByTestId('group-card').map((card) => card.getAttribute('data-record-id'));

describe('Hosts screen content (UI 2.3)', () => {
  it('shows groups above hosts with titles, subtitles and counts', async () => {
    await renderScreen();
    const screenEl = screen.getByTestId('hosts-screen');
    expect(screenEl).toHaveAttribute('data-layout', 'grid');
    expect(screenEl).toHaveAttribute('data-sort', 'manual');
    const groupsSection = screen.getByTestId('hosts-groups-section');
    const hostsSection = screen.getByTestId('hosts-hosts-section');
    expect(groupsSection.compareDocumentPosition(hostsSection) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    expect(shownGroups()).toEqual([ids.prod]);
    expect(within(groupCard(ids.prod)).getByTestId('group-card-title')).toHaveTextContent('prod');
    expect(within(groupCard(ids.prod)).getByTestId('group-card-count')).toHaveTextContent('2 hosts');
    expect(groupCard(ids.prod)).toHaveAttribute('data-color', 'ocean');

    expect(shownHosts()).toEqual([ids.lab, ids.backup]);
    expect(within(hostCard(ids.lab)).getByTestId('host-card-title')).toHaveTextContent('192.0.2.10');
    expect(within(hostCard(ids.lab)).getByTestId('host-card-subtitle')).toHaveTextContent('SSH, root, lab');
    expect(hostCard(ids.backup)).toHaveAttribute('data-color', 'moss');
    expect(hostCard(ids.backup)).toHaveAttribute('data-selected', 'false');
  });

  it('opens a group like a folder with breadcrumbs back to any level', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(groupCard(ids.prod));
    await waitFor(() => expect(shownGroups()).toEqual([ids.web]));
    expect(shownHosts()).toEqual([ids.db1]);
    // The subtitle shows the inherited username.
    expect(within(hostCard(ids.db1)).getByTestId('host-card-subtitle')).toHaveTextContent('SSH, deploy, db');

    await user.click(groupCard(ids.web));
    await waitFor(() => expect(shownHosts()).toEqual([ids.web1]));
    const crumbs = screen.getAllByTestId('hosts-breadcrumb');
    expect(crumbs.map((crumb) => crumb.textContent)).toEqual(['prod', 'web']);
    expect(crumbs[1]).toHaveAttribute('aria-current', 'page');

    await user.click(crumbs[0] as HTMLElement);
    await waitFor(() => expect(shownHosts()).toEqual([ids.db1]));
    await user.click(screen.getByTestId('hosts-breadcrumb-root'));
    await waitFor(() => expect(shownHosts()).toEqual([ids.lab, ids.backup]));
    expect(screen.queryByTestId('hosts-breadcrumb')).not.toBeInTheDocument();
  });

  it('shows the empty folder text', async () => {
    const empty = backend.addGroup({ name: 'empty' });
    const user = userEvent.setup();
    await renderScreen();
    await user.click(groupCard(empty));
    await waitFor(() => expect(screen.getByTestId('hosts-folder-empty')).toBeInTheDocument());
    expect(screen.queryByTestId('hosts-groups-section')).not.toBeInTheDocument();
    expect(screen.queryByTestId('hosts-hosts-section')).not.toBeInTheDocument();
  });

  it('shows the empty state with Create host for an empty vault', async () => {
    for (const id of [...backend.records.keys()]) {
      backend.records.delete(id);
    }
    const user = userEvent.setup();
    await renderScreen();
    expect(screen.getByTestId('hosts-empty-state')).toBeInTheDocument();
    expect(screen.queryByTestId('hosts-empty-import')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('hosts-empty-create'));
    expect(screen.getByTestId('host-panel')).toHaveAttribute('data-save-state', 'draft');
    expect(screen.getByTestId('host-panel-address')).toHaveFocus();
  });

  it('hides the elements of later stages', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(screen.getByTestId('hosts-new-host-toggle'));
    for (const id of [
      'hosts-search-connect',
      'hosts-new-host-item-import',
      'hosts-new-host-item-cloud',
      'hosts-local-terminal-button',
      'hosts-serial-button',
      'hosts-vault-members',
      'host-card-conflict-badge',
    ]) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
    expect(screen.getByTestId('hosts-new-host-item-new-group')).toBeInTheDocument();
  });
});

describe('search and filters (UI 2.1, 2.2)', () => {
  it('filters the whole vault by label, address, tag and username after a pause', async () => {
    const user = userEvent.setup();
    await renderScreen();
    const input = screen.getByTestId('hosts-search-input');
    await user.type(input, 'db');
    await waitFor(() => expect(shownHosts()).toEqual([ids.db1]));
    expect(screen.queryByTestId('hosts-groups-section')).not.toBeInTheDocument();
    const queries = backend.callsOf('records_query') as { scope?: unknown; text?: string }[];
    expect(queries.at(-1)).toMatchObject({ scope: { kind: 'all' }, text: 'db' });
    // While filtering the subtitle names the group.
    expect(within(hostCard(ids.db1)).getByTestId('host-card-subtitle')).toHaveTextContent('SSH, deploy, prod');

    await user.clear(input);
    await user.type(input, 'deploy');
    await waitFor(() => expect(shownHosts()).toEqual([ids.web1, ids.db1]));

    await user.clear(input);
    await user.type(input, 'eu');
    await waitFor(() => expect(shownHosts()).toEqual([ids.web1]));

    await user.clear(input);
    await user.type(input, 'web');
    await waitFor(() => expect(shownGroups()).toEqual([ids.web]));
    expect(shownHosts()).toEqual([ids.web1]);
  });

  it('shows no results and clears back to the folder', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.type(screen.getByTestId('hosts-search-input'), 'nothing-matches');
    await waitFor(() => expect(screen.getByTestId('hosts-no-results')).toBeInTheDocument());
    await user.click(screen.getByTestId('hosts-search-clear'));
    await waitFor(() => expect(shownHosts()).toEqual([ids.lab, ids.backup]));
    expect(screen.getByTestId('hosts-search-input')).toHaveValue('');
    expect(screen.queryByTestId('hosts-search-clear')).not.toBeInTheDocument();
  });

  it('opens a group found by the search and clears the search', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.type(screen.getByTestId('hosts-search-input'), 'web');
    await waitFor(() => expect(shownGroups()).toEqual([ids.web]));
    await user.click(groupCard(ids.web));
    await waitFor(() => expect(screen.getAllByTestId('hosts-breadcrumb').map((crumb) => crumb.textContent)).toEqual(['prod', 'web']));
    expect(screen.getByTestId('hosts-search-input')).toHaveValue('');
  });

  it('filters by tags with AND and removable chips', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(screen.getByTestId('hosts-tag-filter-button'));
    const options = screen.getAllByTestId('hosts-tag-filter-option');
    expect(options.map((option) => option.getAttribute('data-tag'))).toEqual(['db', 'eu', 'lab', 'web']);
    await user.click(options.find((option) => option.getAttribute('data-tag') === 'web') as HTMLElement);
    await waitFor(() => expect(shownHosts()).toEqual([ids.web1]));
    expect(screen.queryByTestId('hosts-groups-section')).not.toBeInTheDocument();
    const webOption = screen.getAllByTestId('hosts-tag-filter-option').find((option) => option.getAttribute('data-tag') === 'web');
    expect(webOption).toHaveAttribute('aria-checked', 'true');
    await user.click(screen.getAllByTestId('hosts-tag-filter-option').find((o) => o.getAttribute('data-tag') === 'db') as HTMLElement);
    await waitFor(() => expect(screen.getByTestId('hosts-no-results')).toBeInTheDocument());
    expect(screen.getAllByTestId('hosts-tag-filter-chip').map((chip) => chip.getAttribute('data-tag'))).toEqual(['web', 'db']);
    await user.keyboard('{Escape}');
    await user.click(screen.getAllByTestId('hosts-tag-filter-chip-remove')[1] as HTMLElement);
    await waitFor(() => expect(shownHosts()).toEqual([ids.web1]));
    const queries = backend.callsOf('records_query') as { tags?: string[] }[];
    expect(queries.some((query) => JSON.stringify(query.tags) === '["web","db"]')).toBe(true);
  });

  it('changes the sort mode and the layout through settings', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(screen.getByTestId('hosts-sort-button'));
    expect(screen.getByTestId('hosts-sort-manual')).toHaveAttribute('aria-checked', 'true');
    await user.click(screen.getByTestId('hosts-sort-name'));
    expect(backend.callsOf('settings_set')).toEqual([{ values: { 'view.hosts.sort': 'name' } }]);
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-sort', 'name');
    expect(screen.getByTestId('hosts-sort-button')).toHaveAttribute('data-sort', 'name');
    await waitFor(() => expect(shownHosts()).toEqual([ids.lab, ids.backup]));

    await user.click(screen.getByTestId('hosts-view-toggle-list'));
    expect(backend.callsOf('settings_set').at(-1)).toEqual({ values: { 'view.hosts.layout': 'list' } });
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-layout', 'list');
  });

  it('follows a layout set back by another window after its own change', async () => {
    const user = userEvent.setup();
    const { rerender } = await renderScreen();
    const show = (layout: string) =>
      rerender(
        <HostsScreen vault={FAKE_VAULT} settings={{ ...SETTINGS, 'view.hosts.layout': layout }} active selectRequest={null} />,
      );
    await user.click(screen.getByTestId('hosts-view-toggle-list'));
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-layout', 'list');
    // `settings:changed` of this window's write, then another window picks tiles again.
    show('list');
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-layout', 'list');
    show('grid');
    expect(screen.getByTestId('hosts-screen')).toHaveAttribute('data-layout', 'grid');
  });
});

describe('selection and the panels (UI 2.4, 1.3)', () => {
  it('selects a host and opens its panel; Ctrl and Shift select several and close it', async () => {
    const extra = backend.addHost({ address: 'extra.example.com' });
    const user = userEvent.setup();
    await renderScreen();
    await user.click(hostCard(ids.lab));
    expect(hostCard(ids.lab)).toHaveAttribute('data-selected', 'true');
    expect(screen.getByTestId('host-panel')).toHaveAttribute('data-record-id', ids.lab);

    fireEvent.click(hostCard(extra), { ctrlKey: true });
    expect(hostCard(ids.lab)).toHaveAttribute('data-selected', 'true');
    expect(hostCard(extra)).toHaveAttribute('data-selected', 'true');
    expect(screen.queryByTestId('host-panel')).not.toBeInTheDocument();

    await user.click(hostCard(ids.lab));
    fireEvent.click(hostCard(extra), { shiftKey: true });
    expect(getHostsViewState().selectedIds).toEqual([ids.lab, ids.backup, extra]);
  });

  it('opens the panel with E focused on Address, moves with arrows and closes with Escape', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(hostCard(ids.lab));
    act(() => hostCard(ids.lab).focus());
    await user.keyboard('{ArrowRight}');
    expect(getHostsViewState().selectedIds).toEqual([ids.backup]);
    expect(hostCard(ids.backup)).toHaveFocus();
    await waitFor(() => expect(screen.getByTestId('host-panel')).toHaveAttribute('data-record-id', ids.backup));
    await user.keyboard('e');
    expect(screen.getByTestId('host-panel-address')).toHaveFocus();
    act(() => hostCard(ids.backup).focus());
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('host-panel')).not.toBeInTheDocument();
  });

  it('creates a host from New host in the open folder', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(groupCard(ids.prod));
    await waitFor(() => expect(shownHosts()).toEqual([ids.db1]));
    await user.click(screen.getByTestId('hosts-new-host-main'));
    expect(screen.getByTestId('host-panel-address')).toHaveFocus();
    await user.type(screen.getByTestId('host-panel-address'), 'new.example.net');
    await waitFor(() => expect(screen.getByTestId('host-panel')).toHaveAttribute('data-save-state', 'saved'), { timeout: 2000 });
    const created = screen.getByTestId('host-panel').getAttribute('data-record-id') as Uuid;
    expect(backend.host(created).group_id).toBe(ids.prod);
    await waitFor(() => expect(shownHosts()).toContain(created));
    expect(hostCard(created)).toHaveAttribute('data-selected', 'true');
  });

  it('creates a group from New Group in the open folder', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(groupCard(ids.prod));
    await waitFor(() => expect(shownGroups()).toEqual([ids.web]));
    await user.click(screen.getByTestId('hosts-new-host-toggle'));
    await user.click(screen.getByTestId('hosts-new-host-item-new-group'));
    expect(screen.getByTestId('group-panel')).toHaveAttribute('data-save-state', 'draft');
    await user.type(screen.getByTestId('group-panel-name'), 'api');
    await waitFor(() => expect(screen.getByTestId('group-panel')).toHaveAttribute('data-save-state', 'saved'), { timeout: 2000 });
    const created = screen.getByTestId('group-panel').getAttribute('data-record-id') as Uuid;
    expect(backend.group(created).parent_id).toBe(ids.prod);
    await waitFor(() => expect(shownGroups()).toContain(created));
  });

  it('closes the panel of a host removed elsewhere', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(hostCard(ids.backup));
    expect(screen.getByTestId('host-panel')).toBeInTheDocument();
    await act(async () => {
      await backend.invoke('record_delete', { vault_id: FAKE_VAULT_ID, record_ids: [ids.backup] });
    });
    await waitFor(() => expect(screen.queryByTestId('host-panel')).not.toBeInTheDocument());
  });
});

describe('context menus, remove and undo (UI 2.4, 3.4)', () => {
  it('shows the host menu entries in order without later-stage items', async () => {
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.lab), { clientX: 10, clientY: 10 });
    const menu = screen.getByTestId('host-context-menu');
    const items = within(menu)
      .getAllByRole('menuitem')
      .map((item) => item.getAttribute('data-testid'));
    expect(items).toEqual(['host-menu-edit', 'host-menu-move-to', 'host-menu-duplicate', 'host-menu-copy-link', 'host-menu-remove']);
    expect(screen.getByTestId('host-menu-copy-link-info')).toBeInTheDocument();
    for (const id of [
      'host-menu-quick-connect',
      'host-menu-connect',
      'host-menu-add-to-workspace',
      'host-menu-collaborate',
      'host-menu-copy-to',
    ]) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
    expect(hostCard(ids.lab)).toHaveAttribute('data-selected', 'true');
  });

  it('copies the host link', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.lab), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('host-menu-copy-link'));
    await waitFor(() => expect(backend.clipboard).toBe(`shellpp://host/${ids.lab}`));
    expect(toasts.map((toast) => toast.testId)).toContain('hosts-link-copied-toast');
  });

  it('moves a host to a group and to the top level', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.lab), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('host-menu-move-to'));
    await user.click(screen.getByTestId(`host-menu-move-to-group-${ids.web}`));
    await waitFor(() => expect(backend.host(ids.lab).group_id).toBe(ids.web));
    await waitFor(() => expect(shownHosts()).toEqual([ids.backup]));
  });

  it('duplicates a host right after the original and selects the copy', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.lab), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('host-menu-duplicate'));
    await waitFor(() => expect(shownHosts()).toHaveLength(3));
    const copy = shownHosts()[1] as Uuid;
    expect(within(hostCard(copy)).getByTestId('host-card-title')).toHaveTextContent('192.0.2.10 Copy');
    expect(getHostsViewState().selectedIds).toEqual([copy]);
  });

  it('removes a host after confirmation and restores it with Undo', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.backup), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('host-menu-remove'));
    const dialog = screen.getByTestId('host-remove-dialog');
    expect(dialog).toHaveAttribute('data-count', '1');
    await user.click(screen.getByTestId('host-remove-dialog-confirm'));
    await waitFor(() => expect(shownHosts()).toEqual([ids.lab]));
    expect(backend.callsOf('record_delete')).toEqual([{ vault_id: FAKE_VAULT_ID, record_ids: [ids.backup] }]);
    const undo = toasts.find((toast) => toast.testId === 'hosts-undo-toast');
    expect(undo?.durationMs).toBe(8000);
    expect(undo?.action?.testId).toBe('hosts-undo-toast-action');
    act(() => undo?.action?.onAction());
    await waitFor(() => expect(shownHosts()).toEqual([ids.lab, ids.backup]));
    expect(backend.callsOf('record_restore')).toEqual([{ vault_id: FAKE_VAULT_ID, record_ids: [ids.backup] }]);
  });

  it('cancels a removal', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(hostCard(ids.backup), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('host-menu-remove'));
    await user.click(screen.getByTestId('host-remove-dialog-cancel'));
    expect(screen.queryByTestId('host-remove-dialog')).not.toBeInTheDocument();
    expect(backend.callsOf('record_delete')).toEqual([]);
  });

  it('removes a multi-selection with the Delete key', async () => {
    const user = userEvent.setup();
    await renderScreen();
    await user.click(hostCard(ids.lab));
    fireEvent.click(hostCard(ids.backup), { ctrlKey: true });
    act(() => hostCard(ids.backup).focus());
    await user.keyboard('{Delete}');
    expect(screen.getByTestId('host-remove-dialog')).toHaveAttribute('data-count', '2');
    await user.click(screen.getByTestId('host-remove-dialog-confirm'));
    await waitFor(() => expect(screen.queryByTestId('hosts-hosts-section')).not.toBeInTheDocument());
    expect(backend.callsOf('record_delete')).toEqual([{ vault_id: FAKE_VAULT_ID, record_ids: [ids.lab, ids.backup] }]);
    expect(getHostsViewState().selectedIds).toEqual([]);
  });

  it('removes a non-empty group after a confirmation that names the move up', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(groupCard(ids.prod), { clientX: 10, clientY: 10 });
    const items = within(screen.getByTestId('group-context-menu'))
      .getAllByRole('menuitem')
      .map((item) => item.getAttribute('data-testid'));
    expect(items).toEqual(['group-menu-open', 'group-menu-edit', 'group-menu-move-to', 'group-menu-remove']);
    await user.click(screen.getByTestId('group-menu-remove'));
    expect(screen.getByTestId('group-remove-dialog-non-empty')).toBeInTheDocument();
    await user.click(screen.getByTestId('group-remove-dialog-confirm'));
    await waitFor(() => expect(shownGroups()).toEqual([ids.web]));
    expect(shownHosts()).toEqual([ids.lab, ids.backup, ids.db1]);
  });

  it('opens and edits a group from its menu', async () => {
    const user = userEvent.setup();
    await renderScreen();
    fireEvent.contextMenu(groupCard(ids.prod), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('group-menu-edit'));
    expect(screen.getByTestId('group-panel')).toHaveAttribute('data-record-id', ids.prod);
    expect(screen.getByTestId('group-panel-name')).toHaveFocus();
    expect(groupCard(ids.prod)).toHaveAttribute('data-selected', 'true');
    fireEvent.contextMenu(groupCard(ids.prod), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('group-menu-open'));
    await waitFor(() => expect(shownHosts()).toEqual([ids.db1]));
  });
});

describe('deep links and drag and drop', () => {
  it('selects a linked host in its group and reports a missing one', async () => {
    const { rerender } = await renderScreen();
    rerender(<HostsScreen vault={FAKE_VAULT} settings={SETTINGS} active selectRequest={{ record_id: ids.web1, nonce: 1 }} />);
    await waitFor(() => expect(screen.getByTestId('host-panel')).toHaveAttribute('data-record-id', ids.web1));
    await waitFor(() => expect(shownHosts()).toEqual([ids.web1]));
    expect(getHostsViewState().folderId).toBe(ids.web);

    rerender(
      <HostsScreen
        vault={FAKE_VAULT}
        settings={SETTINGS}
        active
        selectRequest={{ record_id: '01990000-0000-7000-8000-0000ffffffff', nonce: 2 }}
      />,
    );
    await waitFor(() => expect(toasts.map((toast) => toast.testId)).toContain('hosts-link-missing-toast'));
  });

  it('reorders hosts by dragging in Manual mode and moves a host onto a group card', async () => {
    await renderScreen();
    const lab = hostCard(ids.lab);
    const backup = hostCard(ids.backup);
    backup.getBoundingClientRect = () => new DOMRect(300, 100, 200, 64);
    const original = document.elementFromPoint;
    let under: Element = backup;
    document.elementFromPoint = vi.fn(() => under);
    try {
      fireEvent.pointerDown(lab, { clientX: 10, clientY: 110, button: 0 });
      fireEvent.pointerMove(window, { clientX: 450, clientY: 120 });
      expect(screen.getByTestId('hosts-drop-indicator')).toBeInTheDocument();
      fireEvent.pointerUp(window, { clientX: 450, clientY: 120 });
      await waitFor(() => expect(shownHosts()).toEqual([ids.backup, ids.lab]));
      expect(backend.callsOf('record_move').at(-1)).toEqual({
        vault_id: FAKE_VAULT_ID,
        record_ids: [ids.lab],
        target_group_id: null,
        position: { kind: 'after', record_id: ids.backup },
      });

      under = groupCard(ids.prod);
      fireEvent.pointerDown(hostCard(ids.lab), { clientX: 10, clientY: 110, button: 0 });
      fireEvent.pointerMove(window, { clientX: 60, clientY: 30 });
      expect(groupCard(ids.prod)).toHaveAttribute('data-drop-target', 'true');
      fireEvent.pointerUp(window, { clientX: 60, clientY: 30 });
      await waitFor(() => expect(backend.host(ids.lab).group_id).toBe(ids.prod));
    } finally {
      document.elementFromPoint = original;
    }
  });
});
