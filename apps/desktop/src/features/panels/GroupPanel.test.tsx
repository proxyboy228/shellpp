import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setBackend, type GroupView, type Uuid } from '../../ipc';
import { createFakeBackend, FAKE_VAULT_ID, type FakeBackend } from '../hosts/testing/fakeBackend';
import { GroupPanel, type GroupPanelProps } from './GroupPanel';

let backend: FakeBackend;

beforeEach(() => {
  backend = createFakeBackend();
  setBackend(backend);
});

afterEach(() => {
  setBackend(null);
});

async function groups(): Promise<GroupView[]> {
  const records = await backend.invoke('records_query', { types: ['group'], sort: 'name' });
  return records.filter((record): record is GroupView => record.type === 'group');
}

async function renderPanel(props: Partial<GroupPanelProps> = {}) {
  const all = await groups();
  const full: GroupPanelProps = {
    vaultId: FAKE_VAULT_ID,
    recordId: null,
    view: undefined,
    draftParentId: null,
    groups: all,
    groupsById: new Map(all.map((group) => [group.record_id, group])),
    focusNonce: 1,
    onCreated: vi.fn(),
    onClose: vi.fn(),
    actions: { open: vi.fn(), move: vi.fn(), remove: vi.fn() },
    onMissing: vi.fn(),
    ...props,
  };
  return { ...render(<GroupPanel {...full} />), props: full };
}

async function viewOf(id: Uuid): Promise<GroupView> {
  return (await groups()).find((group) => group.record_id === id) as GroupView;
}

const panel = () => screen.getByTestId('group-panel');

describe('GroupPanel (UI 3.4)', () => {
  it('is a draft until Name is filled, then creates the group in the open folder', async () => {
    const parent = backend.addGroup({ name: 'prod' });
    const user = userEvent.setup();
    const { props } = await renderPanel({ draftParentId: parent });
    expect(screen.getByTestId('group-panel-title')).toHaveTextContent('New group');
    expect(screen.getByTestId('group-panel-name')).toHaveFocus();
    expect(panel()).toHaveAttribute('data-save-state', 'draft');
    // "Required" shows once the field was edited.
    expect(screen.queryByTestId('group-panel-name-error')).not.toBeInTheDocument();
    await user.type(screen.getByTestId('group-panel-name'), 'x');
    await user.clear(screen.getByTestId('group-panel-name'));
    expect(screen.getByTestId('group-panel-name-error')).toBeInTheDocument();
    expect(panel()).toHaveAttribute('data-save-state', 'draft');
    await user.type(screen.getByTestId('group-panel-name'), 'web');
    await waitFor(() => expect(panel()).toHaveAttribute('data-save-state', 'saved'), { timeout: 2000 });
    expect(backend.callsOf('record_save')).toEqual([
      { vault_id: FAKE_VAULT_ID, record_id: null, type: 'group', data: { name: 'web', parent_id: parent } },
    ]);
    expect(props.onCreated).toHaveBeenCalledTimes(1);
    expect(panel()).toHaveAttribute('data-record-id');
  });

  it('shows values inherited from the parent groups as placeholders', async () => {
    const prod = backend.addGroup({ name: 'prod', ssh: { port: 2222, username: 'deploy' } });
    const web = backend.addGroup({ name: 'web', parent_id: prod, ssh: { username: 'www' } });
    const edge = backend.addGroup({ name: 'edge', parent_id: web });
    await renderPanel({ recordId: edge, view: await viewOf(edge), focusNonce: 0 });
    expect(screen.getByTestId('group-panel-ssh-port')).toHaveAttribute('placeholder', '2222 (from group prod)');
    expect(screen.getByTestId('group-panel-ssh-username')).toHaveAttribute('placeholder', 'www (from group web)');
  });

  it('shows the default port placeholder at the top level', async () => {
    await renderPanel();
    expect(screen.getByTestId('group-panel-ssh-port')).toHaveAttribute('placeholder', '22');
    expect(screen.getByTestId('group-panel-ssh-username')).toHaveAttribute('placeholder', '');
  });

  it('saves own SSH values and refuses an invalid port', async () => {
    const id = backend.addGroup({ name: 'prod' });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await viewOf(id), focusNonce: 0 });
    await user.type(screen.getByTestId('group-panel-ssh-port'), '0');
    expect(screen.getByTestId('group-panel-ssh-port-error')).toBeInTheDocument();
    await user.clear(screen.getByTestId('group-panel-ssh-port'));
    await user.type(screen.getByTestId('group-panel-ssh-port'), '2200');
    await user.type(screen.getByTestId('group-panel-ssh-username'), 'ops');
    await waitFor(() => expect(backend.group(id).ssh).toEqual({ port: 2200, username: 'ops' }), { timeout: 2000 });
  });

  it('excludes the group itself and its subtree from the Parent Group choices', async () => {
    const prod = backend.addGroup({ name: 'prod' });
    const web = backend.addGroup({ name: 'web', parent_id: prod });
    backend.addGroup({ name: 'edge', parent_id: web });
    const other = backend.addGroup({ name: 'other' });
    const user = userEvent.setup();
    await renderPanel({ recordId: prod, view: await viewOf(prod), focusNonce: 0 });
    await user.click(screen.getByTestId('group-panel-parent'));
    const choices = screen.getAllByTestId('group-panel-parent-option').map((option) => option.getAttribute('data-group-id'));
    expect(choices).toEqual(['', other]);
    await user.type(screen.getByTestId('group-panel-parent-search'), 'ot');
    expect(screen.getAllByTestId('group-panel-parent-option').map((option) => option.getAttribute('data-group-id'))).toEqual([
      other,
    ]);
  });

  it('offers Open, Edit, Move to and Remove in the menu', async () => {
    const prod = backend.addGroup({ name: 'prod' });
    const user = userEvent.setup();
    const { props } = await renderPanel({ recordId: prod, view: await viewOf(prod), focusNonce: 0 });
    await user.click(screen.getByTestId('group-panel-menu-button'));
    expect(screen.getByTestId('group-panel-menu-open')).toBeInTheDocument();
    expect(screen.getByTestId('group-panel-menu-edit')).toBeInTheDocument();
    expect(screen.getByTestId('group-panel-menu-move-to')).toBeInTheDocument();
    await user.click(screen.getByTestId('group-panel-menu-remove'));
    expect(props.actions.remove).toHaveBeenCalledWith(prod);
    await user.click(screen.getByTestId('group-panel-menu-button'));
    await user.click(screen.getByTestId('group-panel-menu-edit'));
    expect(screen.getByTestId('group-panel-name')).toHaveFocus();
  });

  it('hides the blocks of later stages', async () => {
    await renderPanel();
    for (const id of ['group-panel-vault', 'group-panel-ssh-credentials', 'group-panel-host-chaining']) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
  });
});
