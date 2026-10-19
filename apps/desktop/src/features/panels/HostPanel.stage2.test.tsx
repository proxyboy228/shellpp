// Host panel, stage 2 (UI 3.1, 3.3): the OS icon, the pinned Connect button
// and the "..." menu Connect, the "+" credentials menu with "+ -> Key", the
// linked key field (unlink, missing, inherited). The terminal workspace (S6)
// is replaced with spies; keys use the mock backend's fake parser.

import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setBackend, type Backend, type GroupView, type HostView, type Uuid } from '../../ipc';
import { createMockBackend, type MockBackend } from '../../ipc/mock';
import { openSession } from '../connect/workspaceBridge';
import { createFakeBackend, FAKE_VAULT_ID } from '../hosts/testing/fakeBackend';
import { HostPanel, type HostPanelProps } from './HostPanel';

vi.mock('../connect/workspaceBridge', () => ({
  openSession: vi.fn(() => Promise.resolve(null)),
  openLocalTerminal: vi.fn(() => Promise.resolve(null)),
  activateTab: vi.fn(),
  restoreLastSession: vi.fn(() => Promise.resolve()),
}));

let backend: MockBackend;
let vaultId: string;

beforeEach(async () => {
  vi.mocked(openSession).mockReset();
  vi.mocked(openSession).mockResolvedValue(null);
  backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
  setBackend(backend);
  vaultId = (await backend.invoke('state_snapshot', undefined)).vaults[0]?.vault_id ?? '';
});

afterEach(() => {
  setBackend(null);
});

async function saveHost(data: HostView['data'], using: Backend = backend, vault = vaultId): Promise<HostView> {
  return (await using.invoke('record_save', { vault_id: vault, record_id: null, type: 'host', data })) as HostView;
}

function renderPanel(props: Partial<HostPanelProps>) {
  const groups = props.groups ?? [];
  const all: HostPanelProps = {
    vaultId,
    recordId: null,
    view: undefined,
    draftGroupId: null,
    groups,
    groupsById: new Map(groups.map((group) => [group.record_id, group])),
    allTags: [],
    settings: {},
    showMore: false,
    onShowMoreChange: vi.fn(),
    focusNonce: 0,
    onCreated: vi.fn(),
    onClose: vi.fn(),
    onDuplicate: vi.fn(),
    onRemove: vi.fn(),
    onCreateGroup: vi.fn(() => Promise.resolve(null)),
    onMissing: vi.fn(),
    ...props,
  };
  return render(<HostPanel {...all} />);
}

function keyText(comment: string): string {
  return ['-----BEGIN OPENSSH PRIVATE KEY-----', crypto.randomUUID(), `mock-comment:${comment}`, '-----END OPENSSH PRIVATE KEY-----'].join('\n');
}

async function stored(id: Uuid): Promise<HostView> {
  return (await backend.invoke('record_get', { vault_id: vaultId, record_id: id })) as HostView;
}

describe('HostPanel: Connect (UI 3.1, 3.3)', () => {
  it('is disabled for a draft', () => {
    renderPanel({});
    expect(screen.getByTestId('host-panel-connect')).toBeDisabled();
  });

  it('connects a saved host after saving pending edits', async () => {
    const user = userEvent.setup();
    const host = await saveHost({ address: 'web.example.com' });
    let savedBeforeConnect = false;
    vi.mocked(openSession).mockImplementation(() => {
      // The test's own record_save, then the panel's save of the label.
      savedBeforeConnect = backend.control.calls().filter((command) => command === 'record_save').length === 2;
      return Promise.resolve(null);
    });
    renderPanel({ recordId: host.record_id, view: host });
    await user.type(screen.getByTestId('host-panel-label'), 'Web');
    await user.click(screen.getByTestId('host-panel-connect'));
    await waitFor(() => expect(openSession).toHaveBeenCalledWith({ kind: 'host', vault_id: vaultId, record_id: host.record_id }, { newWindow: false }));
    expect(savedBeforeConnect).toBe(true);
    expect((await stored(host.record_id)).data.label).toBe('Web');
  });

  it('opens a new window with Alt and connects from the "..." menu', async () => {
    const user = userEvent.setup();
    const host = await saveHost({ address: 'web.example.com' });
    renderPanel({ recordId: host.record_id, view: host });
    await user.keyboard('{Alt>}');
    await user.click(screen.getByTestId('host-panel-connect'));
    await user.keyboard('{/Alt}');
    await waitFor(() => expect(openSession).toHaveBeenLastCalledWith(expect.anything(), { newWindow: true }));
    await user.click(screen.getByTestId('host-panel-menu-button'));
    const items = within(screen.getByTestId('host-panel-menu'))
      .getAllByRole('menuitem')
      .map((item) => item.getAttribute('data-testid'));
    expect(items[0]).toBe('host-panel-menu-connect');
    await user.click(screen.getByTestId('host-panel-menu-connect'));
    await waitFor(() => expect(openSession).toHaveBeenCalledTimes(2));
    expect(vi.mocked(openSession).mock.calls[1]?.[1]).toEqual({ newWindow: false });
  });

  it('is disabled while the address is not valid', async () => {
    const user = userEvent.setup();
    const host = await saveHost({ address: 'web.example.com' });
    renderPanel({ recordId: host.record_id, view: host });
    await user.clear(screen.getByTestId('host-panel-address'));
    await user.type(screen.getByTestId('host-panel-address'), 'bad address!');
    expect(screen.getByTestId('host-panel-connect')).toBeDisabled();
  });
});

describe('HostPanel: "+ -> Key" and the linked key (UI 3.1)', () => {
  it('adds a pasted key, links it as ssh.key_id and unlinks it', async () => {
    const user = userEvent.setup();
    const host = await saveHost({ address: 'web.example.com' });
    renderPanel({ recordId: host.record_id, view: host });
    expect(screen.queryByTestId('host-panel-ssh-linked-key')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('host-panel-ssh-credentials-add'));
    const menu = screen.getByTestId('host-panel-credentials-menu');
    expect(within(menu).getAllByRole('menuitem').map((item) => item.getAttribute('data-testid'))).toEqual(['host-panel-credentials-menu-key']);
    for (const id of ['identity', 'certificate', 'fido2']) {
      expect(screen.queryByTestId(`host-panel-credentials-menu-${id}`)).not.toBeInTheDocument();
    }
    await user.click(screen.getByTestId('host-panel-credentials-menu-key'));
    fireEvent.change(await screen.findByTestId('key-add-paste'), { target: { value: keyText('deploy') } });
    await user.click(screen.getByTestId('key-add-submit'));
    await waitFor(() => expect(screen.queryByTestId('key-add-dialog')).not.toBeInTheDocument());

    const linked = await screen.findByTestId('host-panel-ssh-linked-key');
    await waitFor(() => expect(screen.getByTestId('host-panel-ssh-linked-key-label')).toHaveTextContent('deploy'));
    expect(screen.getByTestId('host-panel-ssh-linked-key-type')).toHaveTextContent('ssh-ed25519');
    expect(screen.queryByTestId('host-panel-ssh-credentials-add')).not.toBeInTheDocument();
    const keyId = linked.getAttribute('data-key-id');
    await waitFor(async () => expect((await stored(host.record_id)).data.ssh?.key_id).toBe(keyId), { timeout: 2000 });

    await user.click(screen.getByTestId('host-panel-ssh-linked-key-unlink'));
    expect(screen.queryByTestId('host-panel-ssh-linked-key')).not.toBeInTheDocument();
    expect(screen.getByTestId('host-panel-ssh-credentials-add')).toBeInTheDocument();
    await waitFor(async () => expect((await stored(host.record_id)).data.ssh?.key_id).toBeUndefined(), { timeout: 2000 });
  });

  it('links a saved key from the list', async () => {
    const user = userEvent.setup();
    const key = await backend.invoke('key_import', { vault_id: vaultId, private_key: keyText('ci') });
    const host = await saveHost({ address: 'web.example.com' });
    renderPanel({ recordId: host.record_id, view: host });
    await user.click(screen.getByTestId('host-panel-ssh-credentials-add'));
    await user.click(screen.getByTestId('host-panel-credentials-menu-key'));
    await user.click(await screen.findByTestId('key-add-existing-item'));
    expect(screen.getByTestId('host-panel-ssh-linked-key')).toHaveAttribute('data-key-id', key.key.record_id);
    await waitFor(async () => expect((await stored(host.record_id)).data.ssh?.key_id).toBe(key.key.record_id), { timeout: 2000 });
  });

  it('shows a missing key', async () => {
    const host = await saveHost({ address: 'web.example.com', ssh: { key_id: '01990000-0000-7000-8000-00000000dead' } });
    renderPanel({ recordId: host.record_id, view: host });
    expect(await screen.findByTestId('host-panel-ssh-linked-key-missing')).toHaveTextContent('Missing key');
    expect(screen.getByTestId('host-panel-ssh-linked-key-unlink')).toBeInTheDocument();
  });

  it('shows a key inherited from the group as "from group X" and keeps "+"', async () => {
    const key = await backend.invoke('key_import', { vault_id: vaultId, private_key: keyText('team key') });
    const group: GroupView = {
      type: 'group',
      vault_id: vaultId,
      record_id: '01990000-0000-7000-8000-0000000000aa',
      revision: 1,
      created_at: '2026-10-01T00:00:00.000Z',
      modified_at: '2026-10-01T00:00:00.000Z',
      inherited: {},
      path: [],
      data: { name: 'ops', ssh: { key_id: key.key.record_id } },
      host_count: 0,
      child_count: 0,
    };
    renderPanel({ groups: [group], draftGroupId: group.record_id });
    const field = await screen.findByTestId('host-panel-ssh-linked-key');
    expect(field).toHaveAttribute('data-inherited', 'true');
    expect(screen.getByTestId('host-panel-ssh-linked-key-inherited')).toHaveTextContent('from group ops');
    await waitFor(() => expect(screen.getByTestId('host-panel-ssh-linked-key-label')).toHaveTextContent('team key'));
    expect(screen.queryByTestId('host-panel-ssh-linked-key-unlink')).not.toBeInTheDocument();
    expect(screen.getByTestId('host-panel-ssh-credentials-add')).toBeInTheDocument();
  });
});

describe('HostPanel: OS icon (UI 3.1)', () => {
  it('shows the detected OS family in the address row', async () => {
    const fake = createFakeBackend();
    setBackend(fake);
    const id = fake.addHost({
      address: 'web.example.com',
      os: { family: 'windows', source: 'banner', detected_at: '2026-10-01T00:00:00.000Z' },
    });
    const view = (await fake.invoke('record_get', { vault_id: FAKE_VAULT_ID, record_id: id })) as HostView;
    renderPanel({ vaultId: FAKE_VAULT_ID, recordId: id, view });
    expect(screen.getByTestId('host-panel-os-icon')).toHaveAttribute('data-os-family', 'windows');
  });
});
