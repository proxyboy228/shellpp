import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { IpcError, setBackend, type GroupView, type HostView, type Uuid } from '../../ipc';
import { createFakeBackend, FAKE_VAULT_ID, type FakeBackend } from '../hosts/testing/fakeBackend';
import { HostPanel, type HostPanelProps } from './HostPanel';

let backend: FakeBackend;

beforeEach(() => {
  backend = createFakeBackend({ 'terminal.keepalive_s': 30, 'terminal.default_theme': { builtin: 'graphite' } });
  setBackend(backend);
});

afterEach(() => {
  setBackend(null);
});

async function groupsOf(): Promise<GroupView[]> {
  const records = await backend.invoke('records_query', { types: ['group'], sort: 'name' });
  return records.filter((record): record is GroupView => record.type === 'group');
}

async function hostView(id: Uuid): Promise<HostView> {
  const record = await backend.invoke('record_get', { vault_id: FAKE_VAULT_ID, record_id: id });
  if (record.type !== 'host') {
    throw new Error('not a host');
  }
  return record;
}

async function renderPanel(props: Partial<HostPanelProps> = {}) {
  const groups = await groupsOf();
  const all: HostPanelProps = {
    vaultId: FAKE_VAULT_ID,
    recordId: null,
    view: undefined,
    draftGroupId: null,
    groups,
    groupsById: new Map(groups.map((group) => [group.record_id, group])),
    allTags: [],
    settings: backend.settings,
    showMore: false,
    onShowMoreChange: vi.fn(),
    focusNonce: 1,
    onCreated: vi.fn(),
    onClose: vi.fn(),
    onDuplicate: vi.fn(),
    onRemove: vi.fn(),
    onCreateGroup: vi.fn(() => Promise.resolve(null)),
    onMissing: vi.fn(),
    ...props,
  };
  const result = render(<HostPanel {...all} />);
  return { ...result, props: all };
}

const panel = () => screen.getByTestId('host-panel');
const saveState = () => panel().getAttribute('data-save-state');

describe('HostPanel: draft rule and autosave (UI 1.3, 3.2)', () => {
  it('stays a draft until the address is valid, then creates the host', async () => {
    const user = userEvent.setup();
    const { props } = await renderPanel({ draftGroupId: null });
    const address = screen.getByTestId('host-panel-address');
    expect(address).toHaveFocus();
    expect(screen.getByTestId('host-panel-title')).toHaveTextContent('New host');
    expect(saveState()).toBe('draft');
    expect(panel()).not.toHaveAttribute('data-record-id');

    await user.type(screen.getByTestId('host-panel-label'), 'Primary');
    expect(saveState()).toBe('draft');
    await user.type(address, 'db1.example.com');
    expect(saveState()).toBe('pending');
    await waitFor(() => expect(saveState()).toBe('saved'), { timeout: 2000 });

    const saves = backend.callsOf('record_save');
    expect(saves).toHaveLength(1);
    expect(saves[0]).toEqual({
      vault_id: FAKE_VAULT_ID,
      record_id: null,
      type: 'host',
      data: { address: 'db1.example.com', label: 'Primary' },
    });
    expect(props.onCreated).toHaveBeenCalledTimes(1);
    const created = vi.mocked(props.onCreated).mock.calls[0]?.[0];
    expect(panel()).toHaveAttribute('data-record-id', created?.record_id);
    expect(screen.getByTestId('host-panel-title')).toHaveTextContent('Host details');
  });

  it('creates the host in the open folder', async () => {
    const groupId = backend.addGroup({ name: 'prod' });
    const user = userEvent.setup();
    await renderPanel({ draftGroupId: groupId });
    expect(screen.getByTestId('host-panel-group')).toHaveAttribute('data-group-id', groupId);
    await user.type(screen.getByTestId('host-panel-address'), '192.0.2.10');
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(1), { timeout: 2000 });
    expect(backend.callsOf('record_save')[0]).toMatchObject({ data: { address: '192.0.2.10', group_id: groupId } });
  });

  it('discards a draft without an address when closed', async () => {
    const user = userEvent.setup();
    const { unmount } = await renderPanel();
    await user.type(screen.getByTestId('host-panel-label'), 'Only a label');
    unmount();
    await new Promise((resolve) => setTimeout(resolve, 700));
    expect(backend.callsOf('record_save')).toHaveLength(0);
  });

  it('saves pending changes at once when an existing host closes', async () => {
    const id = backend.addHost({ address: 'web.example.org' });
    const user = userEvent.setup();
    const { unmount } = await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    expect(saveState()).toBe('saved');
    await user.type(screen.getByTestId('host-panel-label'), 'Web');
    unmount();
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(1));
    expect(backend.host(id).label).toBe('Web');
  });

  it('shows an invalid value with its reason and keeps the last valid value', async () => {
    const id = backend.addHost({ address: 'web.example.org', ssh: { port: 2200 } });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    const port = screen.getByTestId('host-panel-ssh-port');
    expect(port).toHaveValue('2200');
    await user.clear(port);
    await user.type(port, '70000');
    expect(screen.getByTestId('host-panel-ssh-port-error')).toBeInTheDocument();
    await user.type(screen.getByTestId('host-panel-label'), 'Web');
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(1), { timeout: 2000 });
    expect(backend.host(id)).toMatchObject({ label: 'Web', ssh: { port: 2200 } });
    expect(screen.getByTestId('host-panel-ssh-port-error')).toBeInTheDocument();
    expect(saveState()).toBe('saved');
  });

  it('keeps the last valid address when the address becomes invalid', async () => {
    const id = backend.addHost({ address: 'web.example.org' });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    const address = screen.getByTestId('host-panel-address');
    await user.clear(address);
    expect(screen.getByTestId('host-panel-address-error')).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 700));
    expect(backend.callsOf('record_save')).toHaveLength(0);
    expect(backend.host(id).address).toBe('web.example.org');
  });

  it('shows the core refusal under the field and saves the other changes', async () => {
    const id = backend.addHost({ address: 'web.example.org' });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    backend.failNext(
      'record_save',
      new IpcError('validation_failed', { errors: [{ field: 'ssh.username', reason: 'not_allowed' }] }),
    );
    await user.type(screen.getByTestId('host-panel-ssh-username'), 'ops');
    await user.type(screen.getByTestId('host-panel-label'), 'Web');
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(2), { timeout: 2000 });
    expect(screen.getByTestId('host-panel-ssh-username-error')).toBeInTheDocument();
    expect(backend.host(id).label).toBe('Web');
    expect(backend.host(id).ssh?.username).toBeUndefined();
  });
});

describe('HostPanel: fields', () => {
  it('shows inherited port and username placeholders "from group X"', async () => {
    const prod = backend.addGroup({ name: 'prod', ssh: { port: 2222, username: 'deploy' } });
    const web = backend.addGroup({ name: 'web', parent_id: prod, ssh: { username: 'www' } });
    const id = backend.addHost({ address: 'web1.example.com', group_id: web });
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    expect(screen.getByTestId('host-panel-ssh-port')).toHaveAttribute('placeholder', '2222 (from group prod)');
    expect(screen.getByTestId('host-panel-ssh-username')).toHaveAttribute('placeholder', 'www (from group web)');
  });

  it('shows the default port placeholder without inheritance and follows the Parent Group choice', async () => {
    const prod = backend.addGroup({ name: 'prod', ssh: { port: 2222 } });
    const user = userEvent.setup();
    await renderPanel();
    const port = screen.getByTestId('host-panel-ssh-port');
    expect(port).toHaveAttribute('placeholder', '22');
    await user.click(screen.getByTestId('host-panel-group'));
    const option = screen
      .getAllByTestId('host-panel-group-option')
      .find((element) => element.getAttribute('data-group-id') === prod);
    expect(option).toBeDefined();
    await user.click(option as HTMLElement);
    expect(port).toHaveAttribute('placeholder', '2222 (from group prod)');
  });

  it('creates a Parent Group on the fly', async () => {
    const user = userEvent.setup();
    const onCreateGroup = vi.fn(async (name: string) => backend.addGroup({ name }));
    await renderPanel({ onCreateGroup });
    await user.click(screen.getByTestId('host-panel-group'));
    await user.type(screen.getByTestId('host-panel-group-search'), 'staging');
    await user.click(screen.getByTestId('host-panel-group-create'));
    expect(onCreateGroup).toHaveBeenCalledWith('staging');
    const created = await onCreateGroup.mock.results[0]?.value;
    expect(screen.getByTestId('host-panel-group')).toHaveAttribute('data-group-id', created);
  });

  it('adds tags with Enter and comma, suggests existing tags and removes chips', async () => {
    const user = userEvent.setup();
    await renderPanel({ allTags: ['database', 'prod'] });
    const input = screen.getByTestId('host-panel-tags-input');
    await user.type(input, 'web{Enter}');
    await user.type(input, 'eu,');
    expect(screen.getAllByTestId('host-panel-tag').map((chip) => chip.getAttribute('data-tag'))).toEqual(['web', 'eu']);
    await user.type(input, 'dat');
    await user.click(screen.getByTestId('host-panel-tags-suggestion'));
    expect(screen.getAllByTestId('host-panel-tag').map((chip) => chip.getAttribute('data-tag'))).toEqual(['web', 'eu', 'database']);
    await user.click(screen.getAllByTestId('host-panel-tag-remove')[1] as HTMLElement);
    expect(screen.getAllByTestId('host-panel-tag').map((chip) => chip.getAttribute('data-tag'))).toEqual(['web', 'database']);
  });

  it('marks the chosen color', async () => {
    const user = userEvent.setup();
    await renderPanel();
    expect(screen.getByTestId('host-panel-color-graphite')).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByTestId('host-panel-color-ocean'));
    expect(screen.getByTestId('host-panel-color-ocean')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('host-panel-color-graphite')).toHaveAttribute('aria-pressed', 'false');
  });

  it('renders Show more fields with their placeholders and errors', async () => {
    const user = userEvent.setup();
    await renderPanel({ showMore: true });
    expect(screen.getByTestId('host-panel-agent-forwarding')).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByTestId('host-panel-charset')).toHaveValue('utf-8');
    expect(screen.getByTestId('host-panel-keepalive')).toHaveAttribute('placeholder', '30 (global)');
    expect(screen.getByTestId('host-panel-record-history')).toHaveValue('default');
    expect(screen.getByTestId('host-panel-theme')).toHaveTextContent('Default (Graphite)');
    await user.type(screen.getByTestId('host-panel-keepalive'), '4000');
    expect(screen.getByTestId('host-panel-keepalive-error')).toBeInTheDocument();

    await user.click(screen.getByTestId('host-panel-env-add'));
    await user.click(screen.getByTestId('host-panel-env-add'));
    const names = screen.getAllByTestId('host-panel-env-name');
    await user.type(names[0] as HTMLElement, 'LANG');
    await user.type(names[1] as HTMLElement, 'LANG');
    expect(screen.getByTestId('host-panel-env-name-error')).toBeInTheDocument();

    await user.click(screen.getByTestId('host-panel-theme'));
    await user.click(screen.getByTestId('host-panel-theme-option-paper'));
    expect(screen.getByTestId('host-panel-theme')).toHaveTextContent('Paper');
  });

  it('hides the blocks of later stages', async () => {
    await renderPanel({ showMore: true });
    for (const id of [
      'host-panel-vault',
      'host-panel-share',
      'host-panel-ssh-credentials-add',
      'host-panel-ssh-linked-key',
      'host-panel-identity-merge-hint',
      'host-panel-startup-snippet',
      'host-panel-host-chaining',
      'host-panel-proxy',
      'host-panel-mosh',
      'host-panel-add-telnet',
      'host-panel-connect',
    ]) {
      expect(screen.queryByTestId(id)).not.toBeInTheDocument();
    }
  });
});

describe('HostPanel: password (secret path)', () => {
  it('sends no secrets while the password is untouched and sets it once typed', async () => {
    const id = backend.addHost({ address: 'web.example.org' });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    await user.type(screen.getByTestId('host-panel-label'), 'Web');
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(1), { timeout: 2000 });
    expect(backend.callsOf('record_save')[0]).not.toHaveProperty('secrets');

    const password = screen.getByTestId('host-panel-ssh-password');
    expect(password).toHaveAttribute('type', 'password');
    await user.type(password, 'generated-test-secret');
    await waitFor(() => expect(backend.callsOf('record_save')).toHaveLength(2), { timeout: 2000 });
    expect(backend.callsOf('record_save')[1]).toMatchObject({ secrets: { 'ssh.password': 'generated-test-secret' } });
    expect(backend.records.get(id)?.secrets['ssh.password']).toBe('generated-test-secret');
  });

  it('shows a saved password as "Saved" and reveals it with the toggle', async () => {
    const id = backend.addHost({ address: 'web.example.org' }, { 'ssh.password': 'stored-test-secret' });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    const password = screen.getByTestId('host-panel-ssh-password');
    expect(password).toHaveValue('');
    expect(password).toHaveAttribute('placeholder', 'Saved');
    await user.click(screen.getByTestId('host-panel-ssh-password-toggle'));
    await waitFor(() => expect(password).toHaveValue('stored-test-secret'));
    expect(password).toHaveAttribute('type', 'text');
    expect(backend.callsOf('secret_reveal')).toEqual([{ vault_id: FAKE_VAULT_ID, record_id: id, path: 'ssh.password' }]);
  });

  it('clears the saved password when the field is emptied', async () => {
    const id = backend.addHost({ address: 'web.example.org' }, { 'ssh.password': 'stored-test-secret' });
    const user = userEvent.setup();
    await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    const password = screen.getByTestId('host-panel-ssh-password');
    await user.type(password, 'x');
    await user.clear(password);
    await waitFor(() => expect(backend.callsOf('record_save').length).toBeGreaterThan(0), { timeout: 2000 });
    const last = backend.callsOf('record_save').at(-1);
    expect(last).toMatchObject({ secrets: { 'ssh.password': null } });
    expect(backend.records.get(id)?.secrets['ssh.password']).toBeUndefined();
  });
});

describe('HostPanel: menu', () => {
  it('offers Duplicate and Remove for a saved host and nothing for a draft', async () => {
    const id = backend.addHost({ address: 'web.example.org' });
    const user = userEvent.setup();
    const { props, unmount } = await renderPanel({ recordId: id, view: await hostView(id), focusNonce: 0 });
    await user.click(screen.getByTestId('host-panel-menu-button'));
    expect(screen.getByTestId('host-panel-menu-duplicate')).toBeInTheDocument();
    expect(screen.queryByTestId('host-panel-menu-connect')).not.toBeInTheDocument();
    expect(screen.queryByTestId('host-panel-menu-add-telnet')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('host-panel-menu-remove'));
    expect(props.onRemove).toHaveBeenCalledWith(id);
    unmount();

    await renderPanel();
    expect(screen.queryByTestId('host-panel-menu-button')).not.toBeInTheDocument();
  });

  it('closes with the collapse button', async () => {
    const { props } = await renderPanel();
    act(() => {
      fireEvent.click(screen.getByTestId('host-panel-collapse'));
    });
    expect(props.onClose).toHaveBeenCalled();
  });
});
