// Rules of the in-browser mock backend (plan, sections 5.1-5.8).

import { describe, expect, it } from 'vitest';
import { IpcError } from '../errors';
import type { EventMap, EventName, GroupView, HostView, RecordView } from '../types';
import { createMockBackend, type MockBackend, type MockBackendOptions } from './index';
import { normalizeRecoveryKey } from './ids';

/** A password made at run time (never a fixed credential). */
function newPassword(): string {
  return `pw-${crypto.randomUUID()}`;
}

async function rejection(promise: Promise<unknown>): Promise<IpcError> {
  try {
    await promise;
  } catch (reason) {
    expect(reason).toBeInstanceOf(IpcError);
    return reason as IpcError;
  }
  throw new Error('expected the command to fail');
}

/** Lets queued event dispatches run. */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function collect<E extends EventName>(backend: MockBackend, event: E): EventMap[E][] {
  const received: EventMap[E][] = [];
  void backend.listen(event, (payload) => received.push(payload));
  return received;
}

async function unlockedBackend(options: MockBackendOptions = {}) {
  const password = newPassword();
  const backend = createMockBackend({ initialState: 'unlocked', password, ...options });
  const snapshot = await backend.invoke('state_snapshot', undefined);
  const vaultId = snapshot.vaults[0]?.vault_id ?? '';
  return { backend, password, vaultId };
}

async function saveHost(backend: MockBackend, vaultId: string, data: Record<string, unknown>, secrets?: Record<string, string | null>) {
  return (await backend.invoke('record_save', {
    vault_id: vaultId,
    record_id: null,
    type: 'host',
    data: data as HostView['data'],
    secrets,
  })) as HostView;
}

async function saveGroup(backend: MockBackend, vaultId: string, data: Record<string, unknown>) {
  return (await backend.invoke('record_save', {
    vault_id: vaultId,
    record_id: null,
    type: 'group',
    data: data as GroupView['data'],
  })) as GroupView;
}

const ids = (views: readonly RecordView[]) => views.map((view) => view.record_id);

describe('mock backend: lock states', () => {
  it('starts at first run and accepts only the first-run commands', async () => {
    const backend = createMockBackend();
    expect((await backend.invoke('lock_status', undefined)).state).toBe('first_run');
    expect((await rejection(backend.invoke('unlock', { password: newPassword() }))).code).toBe('wrong_state');
    expect((await rejection(backend.invoke('lock', undefined))).code).toBe('wrong_state');
    expect((await rejection(backend.invoke('records_query', { types: ['host'] }))).code).toBe('locked');
    expect((await backend.invoke('state_snapshot', undefined)).vaults).toEqual([]);
  });

  it('refuses a short password at first run and creates a recovery key in the RK1 format', async () => {
    const backend = createMockBackend();
    const short = await rejection(backend.invoke('first_run_setup', { password: 'short' }));
    expect(short.code).toBe('validation_failed');
    expect(short.fieldReason('password')).toBe('too_short');

    const lockEvents = collect(backend, 'lock:changed');
    const { recovery_key } = await backend.invoke('first_run_setup', { password: newPassword() });
    expect(recovery_key).toMatch(/^RK1(-[0-9A-HJKMNP-TV-Z]{5}){11}$/);
    await flush();
    expect(lockEvents).toEqual([expect.objectContaining({ locked: false, state: 'unlocked' })]);
    expect((await rejection(backend.invoke('first_run_setup', { password: newPassword() }))).code).toBe('wrong_state');
  });

  it('unlocks with the right password only and locks again', async () => {
    const password = newPassword();
    const backend = createMockBackend({ initialState: 'locked', password });
    expect((await rejection(backend.invoke('unlock', { password: newPassword() }))).code).toBe('invalid_password');
    expect((await backend.invoke('unlock', { password })).state).toBe('unlocked');
    expect((await backend.invoke('lock', undefined)).state).toBe('locked');
    expect(backend.control.lockState()).toBe('locked');
  });

  it('recovers with the recovery key and a new password', async () => {
    const password = newPassword();
    const backend = createMockBackend({ initialState: 'locked', password });
    const key = backend.control.recoveryKey() ?? '';
    const malformed = await rejection(
      backend.invoke('recovery_unlock', { recovery_key: 'RK1-NOPE', new_password: 'short' }),
    );
    expect(malformed.fieldErrors).toEqual([
      { field: 'recovery_key', reason: 'invalid' },
      { field: 'new_password', reason: 'too_short' },
    ]);
    // A well-formed key that differs in one character.
    const other = `${key.slice(0, 4)}${key[4] === 'A' ? 'B' : 'A'}${key.slice(5)}`;
    const wrong = await rejection(backend.invoke('recovery_unlock', { recovery_key: other, new_password: newPassword() }));
    expect(wrong.code).toBe('invalid_recovery_key');

    const next = newPassword();
    // Case, spaces and hyphens do not matter.
    const typed = key.toLowerCase().replace(/-/g, ' ');
    expect((await backend.invoke('recovery_unlock', { recovery_key: typed, new_password: next })).state).toBe('unlocked');
    await backend.invoke('lock', undefined);
    expect((await rejection(backend.invoke('unlock', { password }))).code).toBe('invalid_password');
    expect((await backend.invoke('unlock', { password: next })).state).toBe('unlocked');
  });

  it('changes the password, shows and reissues the recovery key', async () => {
    const { backend, password } = await unlockedBackend();
    expect((await rejection(backend.invoke('password_change', { current_password: 'x', new_password: newPassword() }))).code).toBe(
      'invalid_password',
    );
    const tooShort = await rejection(backend.invoke('password_change', { current_password: password, new_password: 'short' }));
    expect(tooShort.fieldReason('new_password')).toBe('too_short');
    const next = newPassword();
    await backend.invoke('password_change', { current_password: password, new_password: next });

    const shown = await backend.invoke('recovery_key_show', { password: next });
    expect(shown.recovery_key).toBe(backend.control.recoveryKey());
    const reissued = await backend.invoke('recovery_key_reissue', { password: next });
    expect(reissued.recovery_key).not.toBe(shown.recovery_key);

    await backend.invoke('lock', undefined);
    const old = await rejection(backend.invoke('recovery_unlock', { recovery_key: shown.recovery_key, new_password: newPassword() }));
    expect(old.code).toBe('invalid_recovery_key');
  });

  it('turns remember on this device on and off, and refuses without a keychain', async () => {
    const { backend } = await unlockedBackend();
    expect((await backend.invoke('remember_device_set', { enabled: true })).remember_device_enabled).toBe(true);
    expect((await backend.invoke('remember_device_set', { enabled: false })).remember_device_enabled).toBe(false);
    backend.control.setRememberDeviceAvailable(false);
    expect((await rejection(backend.invoke('remember_device_set', { enabled: true }))).code).toBe('unavailable');
  });

  it('auto-locks after the interval without activity and never with "Never"', async () => {
    const { backend } = await unlockedBackend({ settings: { 'security.auto_lock_minutes': 1 } });
    backend.control.advanceClock(50_000);
    await backend.invoke('activity', undefined);
    backend.control.advanceClock(50_000);
    expect(backend.control.lockState()).toBe('unlocked');
    backend.control.advanceClock(11_000);
    expect(backend.control.lockState()).toBe('locked');

    const never = await unlockedBackend({ settings: { 'security.auto_lock_minutes': null } });
    never.backend.control.advanceClock(24 * 3600_000);
    expect(never.backend.control.lockState()).toBe('unlocked');
  });

  it('copies the recovery key only within the grant and clears the clipboard after 30 s', async () => {
    const backend = createMockBackend();
    const { recovery_key } = await backend.invoke('first_run_setup', { password: newPassword() });
    await backend.invoke('secret_copy', { kind: 'recovery_key' });
    expect(backend.control.clipboard()).toBe(recovery_key);
    backend.control.advanceClock(31_000);
    expect(backend.control.clipboard()).toBeNull();
    backend.control.advanceClock(5 * 60_000);
    expect((await rejection(backend.invoke('secret_copy', { kind: 'recovery_key' }))).code).toBe('wrong_state');
  });

  it('fails every command but app_info and window_open after a startup error', async () => {
    const backend = createMockBackend({ startupError: 'newer_schema' });
    expect((await rejection(backend.invoke('state_snapshot', undefined))).code).toBe('newer_schema');
    expect((await backend.invoke('app_info', undefined)).name).toBe('Shell++');
    expect((await backend.invoke('window_open', undefined)).label).toBe('w-2');
  });
});

describe('mock backend: record_save', () => {
  it('creates a host with the normalized data, revision 1 and an order key', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const view = await saveHost(backend, vaultId, {
      address: '  [2001:db8::1]  ',
      label: '',
      tags: ['Prod', 'prod', 'web'],
      charset: 'UTF-8',
      future_member: { kept: true },
      order_key: 'ignored',
    });
    expect(view.revision).toBe(1);
    expect(view.data.address).toBe('2001:db8::1');
    expect(view.data.label).toBeUndefined();
    expect(view.data.tags).toEqual(['Prod', 'web']);
    expect(view.data.charset).toBe('utf-8');
    expect(view.data.future_member).toEqual({ kept: true });
    expect(view.data.order_key).not.toBe('ignored');
    expect(view.record_id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it('reports every invalid field together and writes nothing', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const error = await rejection(
      saveHost(backend, vaultId, {
        address: 'user@host',
        color: 'purple',
        ssh: { port: 70000, env: [{ name: 'A', value: '1' }, { name: 'A', value: '2' }], keepalive_s: 5000 },
      }),
    );
    expect(error.code).toBe('validation_failed');
    expect(error.fieldErrors).toEqual(
      expect.arrayContaining([
        { field: 'address', reason: 'invalid' },
        { field: 'color', reason: 'invalid' },
        { field: 'ssh.port', reason: 'out_of_range' },
        { field: 'ssh.env.1.name', reason: 'duplicate' },
        { field: 'ssh.keepalive_s', reason: 'out_of_range' },
      ]),
    );
    expect(await backend.invoke('records_query', { types: ['host'] })).toEqual([]);
    expect((await rejection(saveHost(backend, vaultId, {}))).fieldReason('address')).toBe('required');
  });

  it('keeps secrets out of data and applies set, keep and clear', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const inData = await rejection(saveHost(backend, vaultId, { address: 'a.example.com', ssh: { password: 'x' } }));
    expect(inData.fieldReason('ssh.password')).toBe('not_allowed');
    const unknown = await rejection(saveHost(backend, vaultId, { address: 'a.example.com' }, { 'ssh.other': 'x' }));
    expect(unknown.fieldReason('ssh.other')).toBe('not_allowed');

    const secret = newPassword();
    const created = await saveHost(backend, vaultId, { address: 'a.example.com' }, { 'ssh.password': secret });
    expect(created.secrets_set).toEqual(['ssh.password']);
    expect(JSON.stringify(created)).not.toContain(secret);
    const ref = { vault_id: vaultId, record_id: created.record_id };
    expect((await backend.invoke('secret_reveal', { ...ref, path: 'ssh.password' })).value).toBe(secret);

    const kept = (await backend.invoke('record_save', { ...ref, type: 'host', data: { ...created.data, label: 'A' } })) as HostView;
    expect(kept.revision).toBe(2);
    expect(kept.secrets_set).toEqual(['ssh.password']);
    const cleared = (await backend.invoke('record_save', {
      ...ref,
      type: 'host',
      data: kept.data,
      secrets: { 'ssh.password': null },
    })) as HostView;
    expect(cleared.secrets_set).toEqual([]);
    expect((await rejection(backend.invoke('secret_reveal', { ...ref, path: 'ssh.password' }))).code).toBe('not_found');
  });

  it('refuses group cycles and unknown records', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const parent = await saveGroup(backend, vaultId, { name: 'Parent' });
    const child = await saveGroup(backend, vaultId, { name: 'Child', parent_id: parent.record_id });
    const cycle = await rejection(
      backend.invoke('record_save', {
        vault_id: vaultId,
        record_id: parent.record_id,
        type: 'group',
        data: { ...parent.data, parent_id: child.record_id },
      }),
    );
    expect(cycle.fieldReason('parent_id')).toBe('cycle');
    expect((await rejection(saveGroup(backend, vaultId, { name: '  ' }))).fieldReason('name')).toBe('required');
    const missing = await rejection(
      backend.invoke('record_save', { vault_id: vaultId, record_id: crypto.randomUUID(), type: 'host', data: { address: 'x' } }),
    );
    expect(missing.code).toBe('not_found');
  });

  it('emits records:changed with an increasing version', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const events = collect(backend, 'records:changed');
    const first = await saveHost(backend, vaultId, { address: 'a.example.com' });
    const second = await saveHost(backend, vaultId, { address: 'b.example.com' });
    await flush();
    expect(events.map((event) => event.record_ids)).toEqual([[first.record_id], [second.record_id]]);
    expect(events[1].v).toBeGreaterThan(events[0].v);
    expect(events[0].classes).toEqual([2]);
    expect((await backend.invoke('state_snapshot', undefined)).v).toBe(events[1].v);
  });
});

describe('mock backend: records_query', () => {
  async function tree() {
    const context = await unlockedBackend();
    const { backend, vaultId } = context;
    const prod = await saveGroup(backend, vaultId, { name: 'Production', ssh: { username: 'deploy', port: 2222 } });
    const db = await saveGroup(backend, vaultId, { name: 'Databases', parent_id: prod.record_id, ssh: { port: 2200 } });
    const web = await saveHost(backend, vaultId, { address: '203.0.113.10', label: 'web-1', group_id: prod.record_id, tags: ['web', 'prod'] });
    const dbHost = await saveHost(backend, vaultId, { address: 'db1.example.com', group_id: db.record_id, tags: ['db', 'prod'] });
    const bastion = await saveHost(backend, vaultId, { address: '198.51.100.7', label: 'Bastion', ssh: { username: 'admin' } });
    return { ...context, prod, db, web, dbHost, bastion };
  }

  it('returns groups first, then hosts, per scope', async () => {
    const { backend, prod, db, web, bastion } = await tree();
    const top = await backend.invoke('records_query', { types: ['host', 'group'], scope: { kind: 'top_level' } });
    expect(ids(top)).toEqual([prod.record_id, bastion.record_id]);
    const inProd = await backend.invoke('records_query', {
      types: ['host', 'group'],
      scope: { kind: 'group', group_id: prod.record_id },
    });
    expect(ids(inProd)).toEqual([db.record_id, web.record_id]);
  });

  it('computes inherited values, paths and counts', async () => {
    const { backend, vaultId, prod, db, dbHost } = await tree();
    const host = (await backend.invoke('record_get', { vault_id: vaultId, record_id: dbHost.record_id })) as HostView;
    expect(host.inherited.port).toEqual({ value: 2200, from_group_id: db.record_id });
    expect(host.inherited.username).toEqual({ value: 'deploy', from_group_id: prod.record_id });
    expect(host.path).toEqual([prod.record_id, db.record_id]);
    const group = (await backend.invoke('record_get', { vault_id: vaultId, record_id: prod.record_id })) as GroupView;
    expect(group.host_count).toBe(2);
    expect(group.child_count).toBe(2);
    expect(group.path).toEqual([]);
  });

  it('filters by text (label, address, tag, own and inherited username) and by tags', async () => {
    const { backend, prod, web, dbHost, bastion } = await tree();
    const search = async (text: string) => ids(await backend.invoke('records_query', { types: ['host', 'group'], text }));
    expect(await search('WEB-1')).toEqual([web.record_id]);
    expect(await search('db1.example')).toEqual([dbHost.record_id]);
    expect(await search('admin')).toEqual([bastion.record_id]);
    expect(await search('deploy')).toEqual([web.record_id, dbHost.record_id]);
    expect(await search('product')).toEqual([prod.record_id]);
    const tagged = await backend.invoke('records_query', { types: ['host', 'group'], tags: ['PROD', 'db'] });
    expect(ids(tagged)).toEqual([dbHost.record_id]);
  });

  it('sorts by name, created and modified', async () => {
    const { backend, vaultId, web, dbHost, bastion } = await tree();
    const sorted = async (sort: 'name' | 'created' | 'modified') =>
      ids(await backend.invoke('records_query', { types: ['host'], sort }));
    expect(await sorted('name')).toEqual([bastion.record_id, dbHost.record_id, web.record_id]);
    expect(await sorted('created')).toEqual([bastion.record_id, dbHost.record_id, web.record_id]);
    await backend.invoke('record_save', { vault_id: vaultId, record_id: web.record_id, type: 'host', data: web.data });
    expect((await sorted('modified'))[0]).toBe(web.record_id);
  });
});

describe('mock backend: delete, restore, move, duplicate', () => {
  it('moves the children of a removed group one level up and restores within 60 s', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const parent = await saveGroup(backend, vaultId, { name: 'Parent' });
    const group = await saveGroup(backend, vaultId, { name: 'Group', parent_id: parent.record_id });
    const host = await saveHost(backend, vaultId, { address: 'a.example.com', group_id: group.record_id });
    const result = await backend.invoke('record_delete', { vault_id: vaultId, record_ids: [group.record_id] });
    expect(result).toEqual({ deleted: [group.record_id], moved: [host.record_id] });
    const moved = (await backend.invoke('record_get', { vault_id: vaultId, record_id: host.record_id })) as HostView;
    expect(moved.data.group_id).toBe(parent.record_id);

    const restored = await backend.invoke('record_restore', { vault_id: vaultId, record_ids: [group.record_id] });
    expect(ids(restored)).toEqual([group.record_id]);

    await backend.invoke('record_delete', { vault_id: vaultId, record_ids: [host.record_id] });
    backend.control.advanceClock(61_000);
    const late = await rejection(backend.invoke('record_restore', { vault_id: vaultId, record_ids: [host.record_id] }));
    expect(late.code).toBe('not_found');
  });

  it('reorders with before/after and refuses cycles and foreign anchors', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const a = await saveHost(backend, vaultId, { address: 'a.example.com' });
    const b = await saveHost(backend, vaultId, { address: 'b.example.com' });
    const c = await saveHost(backend, vaultId, { address: 'c.example.com' });
    await backend.invoke('record_move', {
      vault_id: vaultId,
      record_ids: [c.record_id],
      target_group_id: null,
      position: { kind: 'before', record_id: a.record_id },
    });
    const order = async () => ids(await backend.invoke('records_query', { types: ['host'], scope: { kind: 'top_level' } }));
    expect(await order()).toEqual([c.record_id, a.record_id, b.record_id]);
    await backend.invoke('record_move', {
      vault_id: vaultId,
      record_ids: [c.record_id],
      position: { kind: 'after', record_id: a.record_id },
    });
    expect(await order()).toEqual([a.record_id, c.record_id, b.record_id]);

    const group = await saveGroup(backend, vaultId, { name: 'G' });
    const sub = await saveGroup(backend, vaultId, { name: 'Sub', parent_id: group.record_id });
    const cycle = await rejection(
      backend.invoke('record_move', { vault_id: vaultId, record_ids: [group.record_id], target_group_id: sub.record_id }),
    );
    expect(cycle.fieldReason('target_group_id')).toBe('cycle');
    const anchor = await rejection(
      backend.invoke('record_move', {
        vault_id: vaultId,
        record_ids: [a.record_id],
        target_group_id: group.record_id,
        position: { kind: 'before', record_id: b.record_id },
      }),
    );
    expect(anchor.fieldReason('position')).toBe('invalid');
    const missing = await rejection(
      backend.invoke('record_move', { vault_id: vaultId, record_ids: [a.record_id], target_group_id: a.record_id }),
    );
    expect(missing.fieldReason('target_group_id')).toBe('missing_ref');
  });

  it('duplicates a host right after the original with " Copy" and its secrets', async () => {
    const { backend, vaultId } = await unlockedBackend();
    const secret = newPassword();
    const a = await saveHost(backend, vaultId, { address: 'a.example.com', import_key: 'k' }, { 'ssh.password': secret });
    const b = await saveHost(backend, vaultId, { address: 'b.example.com', label: 'B' });
    const copy = (await backend.invoke('record_duplicate', { vault_id: vaultId, record_id: a.record_id })) as HostView;
    expect(copy.data.label).toBe('a.example.com Copy');
    expect(copy.data.import_key).toBeUndefined();
    expect(copy.secrets_set).toEqual(['ssh.password']);
    const order = ids(await backend.invoke('records_query', { types: ['host'] }));
    expect(order).toEqual([a.record_id, copy.record_id, b.record_id]);
    const group = await saveGroup(backend, vaultId, { name: 'G' });
    const refused = await rejection(backend.invoke('record_duplicate', { vault_id: vaultId, record_id: group.record_id }));
    expect(refused.code).toBe('validation_failed');
  });
});

describe('mock backend: settings, notifications, clipboard, links', () => {
  it('returns every key with defaults, validates and reports changed keys', async () => {
    const { backend } = await unlockedBackend();
    const settings = await backend.invoke('settings_get', undefined);
    expect(settings['security.auto_lock_minutes']).toBe(15);
    expect(settings['view.hosts.sort']).toBe('manual');
    expect(settings['view.keychain.sort']).toBe('name');
    expect(settings['view.details_panel_width']).toBe(380);

    const events = collect(backend, 'settings:changed');
    const bad = await rejection(
      backend.invoke('settings_set', { values: { 'view.details_panel_width': 900, 'no.such.key': 1 } }),
    );
    expect(bad.fieldErrors).toEqual([
      { field: 'view.details_panel_width', reason: 'out_of_range' },
      { field: 'no.such.key', reason: 'invalid' },
    ]);
    await backend.invoke('settings_set', { values: { 'security.auto_lock_minutes': null, 'appearance.ui_zoom': 1 } });
    await flush();
    expect(events.map((event) => event.keys)).toEqual([['security.auto_lock_minutes']]);
    await backend.invoke('lock', undefined);
    expect((await rejection(backend.invoke('settings_set', { values: {} }))).code).toBe('locked');
    expect((await backend.invoke('settings_get', undefined))['security.auto_lock_minutes']).toBeNull();
  });

  it('keeps notifications newest first, at most 200, and marks them read', async () => {
    const { backend } = await unlockedBackend();
    const events = collect(backend, 'notifications:changed');
    for (let index = 0; index < 205; index += 1) {
      backend.control.notify('notifications.test', { index: String(index) });
    }
    const list = await backend.invoke('notifications_list', undefined);
    expect(list.items).toHaveLength(200);
    expect(list.items[0].id).toBe(205);
    expect(list.unread).toBe(200);
    const read = await backend.invoke('notifications_mark_read', undefined);
    expect(read.unread).toBe(0);
    await flush();
    expect(events.at(-1)).toEqual(expect.objectContaining({ unread: 0 }));
    expect(events.at(-1)?.latest).toBeUndefined();
    expect(events[0].latest?.message_key).toBe('notifications.test');
  });

  it('opens only https links and writes non-sensitive clipboard text', async () => {
    const { backend } = await unlockedBackend();
    await backend.invoke('external_open_url', { url: 'https://example.com/docs' });
    for (const url of ['http://example.com', 'https://user:pw@example.com', 'file:///etc/passwd', 'not a url']) {
      expect((await rejection(backend.invoke('external_open_url', { url }))).fieldReason('url')).toBe('not_allowed');
    }
    expect(backend.control.openedUrls()).toEqual(['https://example.com/docs']);
    await backend.invoke('clipboard_write_text', { text: 'shellpp://host/x' });
    expect((await backend.invoke('clipboard_read_text', undefined)).text).toBe('shellpp://host/x');
    backend.control.advanceClock(60_000);
    expect(backend.control.clipboard()).toBe('shellpp://host/x');
  });

  it('seeds documentation-only demo fixtures', async () => {
    const { backend } = await unlockedBackend({ fixtures: 'demo' });
    const hosts = (await backend.invoke('records_query', { types: ['host'] })) as HostView[];
    expect(hosts.length).toBeGreaterThan(3);
    for (const host of hosts) {
      expect(host.data.address).toMatch(/(example\.(com|org)$)|^192\.0\.2\.|^198\.51\.100\.|^203\.0\.113\.|^2001:db8:/);
    }
  });
});

describe('recovery key normalization', () => {
  it('ignores case, spaces, hyphens and the prefix and maps I, L, O', () => {
    const key = `RK1-${Array.from({ length: 11 }, () => 'ABCDE').join('-')}`;
    expect(normalizeRecoveryKey(key)).toBe('ABCDE'.repeat(11));
    expect(normalizeRecoveryKey(key.toLowerCase().replace(/-/g, ' '))).toBe('ABCDE'.repeat(11));
    expect(normalizeRecoveryKey('IL0O'.repeat(13) + 'ABC')).toBe('1100'.repeat(13) + 'ABC');
    expect(normalizeRecoveryKey('RK1-ABCDE')).toBeNull();
    expect(normalizeRecoveryKey('U'.repeat(55))).toBeNull();
  });
});
