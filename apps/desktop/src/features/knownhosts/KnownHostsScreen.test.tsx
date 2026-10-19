// Known Hosts screen (UI 10.1) against the mock backend: list, hashed and
// revoked entries, search, view and sort settings, details panel, copy,
// remove. Keys and fingerprints are fake test values.

import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { resetStores, selectRequestStore, viewStore } from '../../app/store';
import { clearToasts, getToasts } from '../../app/toast';
import { setBackend, type SettingsValues, type VaultView } from '../../ipc';
import { createMockBackend, type MockBackend } from '../../ipc/mock';
import { KnownHostsScreen } from './KnownHostsScreen';

let backend: MockBackend;
let vault: VaultView;
let settings: SettingsValues;

beforeEach(async () => {
  resetStores();
  clearToasts();
  backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
  setBackend(backend);
  const snapshot = await backend.invoke('state_snapshot', undefined);
  vault = snapshot.vaults[0] as VaultView;
  settings = snapshot.settings;
});

afterEach(() => {
  setBackend(null);
  resetStores();
  clearToasts();
});

async function renderScreen() {
  const result = render(<KnownHostsScreen vault={vault} settings={settings} active />);
  await waitFor(() => expect(screen.getByTestId('known-hosts-screen')).toHaveAttribute('data-ready', 'true'));
  return result;
}

function seed() {
  const control = backend.control.connect;
  return {
    web: control.addKnownHost({ host: 'web.example.com', key_type: 'ssh-ed25519', public_key: 'AAAAweb' }),
    lab: control.addKnownHost({ host: '192.0.2.30', port: 2222, key_type: 'ecdsa-sha2-nistp256', public_key: 'AAAAlab' }),
    hashed: control.addKnownHost({ host: '|1|c2FsdA==|aGFzaA==', key_type: 'ssh-rsa', public_key: 'AAAAhash' }),
    revoked: control.addKnownHost({ host: 'old.example.org', key_type: 'ssh-ed25519', public_key: 'AAAAold', revoked: true }),
  };
}

const cards = () => screen.queryAllByTestId('known-host-card');
const titles = () => cards().map((card) => within(card).getByTestId('known-host-card-title').textContent);
const card = (id: string) => cards().find((element) => element.getAttribute('data-record-id') === id) as HTMLElement;

describe('KnownHostsScreen: list (UI 10.1)', () => {
  it('shows the empty state without entries', async () => {
    await renderScreen();
    expect(screen.getByTestId('known-hosts-empty')).toBeInTheDocument();
    expect(cards()).toEqual([]);
  });

  it('shows OpenSSH addresses, hashed entries and revoked badges, sorted by name', async () => {
    const ids = seed();
    await renderScreen();
    expect(titles()).toEqual(['[192.0.2.30]:2222', 'old.example.org', 'web.example.com', 'Hashed entry']);
    expect(card(ids.hashed)).toHaveAttribute('data-hashed', 'true');
    expect(within(card(ids.hashed)).getByTestId('known-host-card-subtitle')).toHaveTextContent('ssh-rsa');
    expect(card(ids.revoked)).toHaveAttribute('data-revoked', 'true');
    expect(within(card(ids.revoked)).getByTestId('known-host-card-revoked-badge')).toHaveTextContent('Revoked');
    expect(within(card(ids.web)).queryByTestId('known-host-card-revoked-badge')).not.toBeInTheDocument();
    // Import and Export are stage 3.
    expect(screen.queryByTestId('known-hosts-import')).not.toBeInTheDocument();
    expect(screen.queryByTestId('known-hosts-export')).not.toBeInTheDocument();
  });

  it('searches by address, key type and fingerprint', async () => {
    const user = userEvent.setup();
    seed();
    await renderScreen();
    const search = screen.getByTestId('known-hosts-search-input');
    // UI 10.1: the search sits on the right of the action bar with the view toggle and sorting.
    expect(screen.getByTestId('known-hosts-view-toggle').parentElement).toContainElement(search);
    await user.type(search, 'ECDSA');
    expect(titles()).toEqual(['[192.0.2.30]:2222']);
    await user.clear(search);
    const fingerprint = (await backend.invoke('known_hosts_list', undefined)).find((entry) => entry.host === 'web.example.com')
      ?.fingerprint as string;
    await user.type(search, fingerprint.slice(7, 20));
    expect(titles()).toEqual(['web.example.com']);
    await user.clear(search);
    await user.type(search, 'nothing-like-this');
    expect(screen.getByTestId('known-hosts-no-results')).toBeInTheDocument();
    await user.click(screen.getByTestId('known-hosts-search-clear'));
    expect(cards()).toHaveLength(4);
  });

  it('switches view and sort and stores them in the settings', async () => {
    const user = userEvent.setup();
    seed();
    await renderScreen();
    const root = screen.getByTestId('known-hosts-screen');
    expect(root).toHaveAttribute('data-layout', 'grid');
    expect(root).toHaveAttribute('data-sort', 'name');
    await user.click(screen.getByTestId('known-hosts-view-toggle-list'));
    expect(root).toHaveAttribute('data-layout', 'list');
    await user.click(screen.getByTestId('known-hosts-sort-button'));
    await user.click(screen.getByTestId('known-hosts-sort-created'));
    expect(root).toHaveAttribute('data-sort', 'created');
    expect(screen.getByTestId('known-hosts-sort-button')).toHaveAttribute('data-sort', 'created');
    // Newest first.
    expect(titles()[0]).toBe('old.example.org');
    await waitFor(async () => {
      const stored = await backend.invoke('settings_get', undefined);
      expect(stored['view.known_hosts.layout']).toBe('list');
      expect(stored['view.known_hosts.sort']).toBe('created');
    });
  });

  it('reloads when known hosts change', async () => {
    await renderScreen();
    act(() => {
      backend.control.connect.addKnownHost({ host: 'new.example.net', key_type: 'ssh-ed25519', public_key: 'AAAAnew' });
    });
    await waitFor(() => expect(titles()).toEqual(['new.example.net']));
  });
});

describe('KnownHostsScreen: details, copy, remove', () => {
  it('opens the details panel with every field and the saved hosts with this address', async () => {
    const user = userEvent.setup();
    const ids = seed();
    const host = await backend.invoke('record_save', {
      vault_id: vault.vault_id,
      record_id: null,
      type: 'host',
      data: { address: '192.0.2.30', label: 'Lab box', ssh: { port: 2222 } },
    });
    await renderScreen();
    await user.click(card(ids.lab));
    const panel = await screen.findByTestId('known-host-panel');
    expect(panel).toHaveAttribute('data-record-id', ids.lab);
    expect(card(ids.lab)).toHaveAttribute('data-selected', 'true');
    expect(screen.getByTestId('known-host-panel-address')).toHaveTextContent('[192.0.2.30]:2222');
    expect(screen.getByTestId('known-host-panel-port')).toHaveTextContent('2222');
    expect(screen.getByTestId('known-host-panel-key-type')).toHaveTextContent('ecdsa-sha2-nistp256');
    expect(screen.getByTestId('known-host-panel-fingerprint')).toHaveTextContent(/^SHA256:/);
    expect(screen.getByTestId('known-host-panel-added').textContent).not.toBe('');
    const link = await screen.findByTestId('known-host-panel-host');
    expect(link).toHaveAttribute('data-record-id', host.record_id);
    expect(link).toHaveTextContent('Lab box');

    await user.click(link);
    expect(selectRequestStore.get()?.record_id).toBe(host.record_id);
    expect(viewStore.get().vaultsSection).toBe('hosts');
  });

  it('shows "no saved host" for an entry without hosts and the port of a hashed entry', async () => {
    const user = userEvent.setup();
    const ids = seed();
    await renderScreen();
    await user.click(card(ids.hashed));
    expect(await screen.findByTestId('known-host-panel-hosts-empty')).toBeInTheDocument();
    expect(screen.getByTestId('known-host-panel-address')).toHaveTextContent('Hashed entry');
    expect(screen.getByTestId('known-host-panel-port')).toHaveTextContent('Part of the hashed entry');
  });

  it('copies the fingerprint from the panel and from the context menu', async () => {
    const user = userEvent.setup();
    const ids = seed();
    await renderScreen();
    const expected = (await backend.invoke('known_hosts_list', undefined)).find((entry) => entry.record_id === ids.web)?.fingerprint;
    await user.click(card(ids.web));
    await user.click(await screen.findByTestId('known-host-panel-fingerprint-copy'));
    await waitFor(() => expect(backend.control.clipboard()).toBe(expected));
    expect(getToasts().map((toast) => toast.testId)).toContain('known-hosts-fingerprint-copied-toast');

    backend.control.setClipboard(null);
    fireEvent.contextMenu(card(ids.lab), { clientX: 10, clientY: 10 });
    expect(screen.getByTestId('known-host-context-menu')).toBeInTheDocument();
    await user.click(screen.getByTestId('known-host-menu-copy-fingerprint'));
    const lab = (await backend.invoke('known_hosts_list', undefined)).find((entry) => entry.record_id === ids.lab)?.fingerprint;
    await waitFor(() => expect(backend.control.clipboard()).toBe(lab));
  });

  it('removes an entry after confirmation (panel and context menu)', async () => {
    const user = userEvent.setup();
    const ids = seed();
    await renderScreen();
    await user.click(card(ids.web));
    await user.click(await screen.findByTestId('known-host-panel-remove'));
    const dialog = screen.getByTestId('known-host-remove-dialog');
    expect(dialog).toHaveTextContent('web.example.com');
    await user.click(screen.getByTestId('known-host-remove-dialog-cancel'));
    expect(card(ids.web)).toBeDefined();

    await user.click(screen.getByTestId('known-host-panel-remove'));
    await user.click(screen.getByTestId('known-host-remove-dialog-confirm'));
    await waitFor(() => expect(card(ids.web)).toBeUndefined());
    expect(screen.queryByTestId('known-host-panel')).not.toBeInTheDocument();

    fireEvent.contextMenu(card(ids.lab), { clientX: 10, clientY: 10 });
    await user.click(screen.getByTestId('known-host-menu-remove'));
    await user.click(screen.getByTestId('known-host-remove-dialog-confirm'));
    await waitFor(() => expect(card(ids.lab)).toBeUndefined());
    expect((await backend.invoke('known_hosts_list', undefined)).map((entry) => entry.record_id)).toEqual(
      expect.not.arrayContaining([ids.web, ids.lab]),
    );
  });

  it('closes the panel with Escape', async () => {
    const user = userEvent.setup();
    const ids = seed();
    await renderScreen();
    await user.click(card(ids.web));
    await screen.findByTestId('known-host-panel');
    card(ids.web).focus();
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('known-host-panel')).not.toBeInTheDocument();
  });
});
