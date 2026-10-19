import { describe, expect, it } from 'vitest';
import { HOST_FIELDS, GROUP_FIELDS, type HostForm } from './fields';
import {
  activeServerErrors,
  buildData,
  formFromData,
  getPath,
  jsonEqual,
  setPath,
  syncForm,
  toServerErrors,
  validateForm,
} from './form';

const stored = {
  address: 'db1.example.com',
  label: 'DB primary',
  tags: ['prod'],
  ssh: { port: 2222, identity_id: '01990000-0000-7000-8000-000000000099' },
  os: { family: 'linux', source: 'script', detected_at: '2026-10-18T22:31:05.123Z' },
  future_member: { kept: true },
};

function edit(form: HostForm, patch: Partial<HostForm>): HostForm {
  return { ...form, ...patch };
}

describe('form engine', () => {
  it('reads and writes nested paths without touching the input', () => {
    const data = { ssh: { port: 22, username: 'ops' } };
    expect(getPath(data, 'ssh.port')).toBe(22);
    const next = setPath(data, 'ssh.port', undefined);
    expect(next).toEqual({ ssh: { username: 'ops' } });
    expect(data.ssh.port).toBe(22);
    expect(setPath({}, 'ssh.username', 'root')).toEqual({ ssh: { username: 'root' } });
  });

  it('treats absent and null members as equal', () => {
    expect(jsonEqual({ a: 1, b: null }, { a: 1 })).toBe(true);
    expect(jsonEqual([1, { x: undefined }], [1, {}])).toBe(true);
    expect(jsonEqual({ a: [1] }, { a: [2] })).toBe(false);
  });

  it('is not dirty for an untouched record whose members have defaults', () => {
    const form = formFromData(HOST_FIELDS, stored);
    expect(form.color).toBe('graphite');
    expect(form.charset).toBe('utf-8');
    expect(form.port).toBe('2222');
    expect(buildData(HOST_FIELDS, stored, validateForm(HOST_FIELDS, form)).changed).toBe(false);
  });

  it('applies valid edits and keeps members the panel does not edit', () => {
    const form = edit(formFromData(HOST_FIELDS, stored), { label: '  DB replica ', username: 'ops' });
    const { data, changed } = buildData(HOST_FIELDS, stored, validateForm(HOST_FIELDS, form));
    expect(changed).toBe(true);
    expect(data).toEqual({
      ...stored,
      label: 'DB replica',
      ssh: { port: 2222, identity_id: stored.ssh.identity_id, username: 'ops' },
    });
  });

  it('keeps the last valid value of an invalid field (UI 1.3)', () => {
    const form = edit(formFromData(HOST_FIELDS, stored), { port: '70000', label: 'renamed' });
    const validation = validateForm(HOST_FIELDS, form);
    expect(validation.errors).toEqual([{ path: 'ssh.port', reason: 'out_of_range' }]);
    const { data } = buildData(HOST_FIELDS, stored, validation);
    expect(getPath(data, 'ssh.port')).toBe(2222);
    expect(data.label).toBe('renamed');
  });

  it('removes a member whose field was emptied', () => {
    const form = edit(formFromData(HOST_FIELDS, stored), { label: '', port: '' });
    const { data } = buildData(HOST_FIELDS, stored, validateForm(HOST_FIELDS, form));
    expect('label' in data).toBe(false);
    expect(getPath(data, 'ssh.port')).toBeUndefined();
  });

  it('skips fields refused by the core while they hold the refused value', () => {
    const form = edit(formFromData(HOST_FIELDS, stored), { username: 'bad', label: 'ok' });
    const refused = toServerErrors(HOST_FIELDS, form, [{ field: 'ssh.username', reason: 'invalid' }]);
    expect(refused).toHaveLength(1);
    const skip = new Set(activeServerErrors(refused, form).map((entry) => entry.key));
    const { data } = buildData(HOST_FIELDS, stored, validateForm(HOST_FIELDS, form), skip);
    expect(getPath(data, 'ssh.username')).toBeUndefined();
    expect(data.label).toBe('ok');
    expect(activeServerErrors(refused, edit(form, { username: 'better' }))).toEqual([]);
  });

  it('takes external changes into fields that were in sync and keeps typed ones', () => {
    const form = edit(formFromData(HOST_FIELDS, stored), { label: 'typed here ' });
    const external = { ...stored, label: 'changed elsewhere', notes: 'from another window' };
    const synced = syncForm(HOST_FIELDS, form, stored, external);
    expect(synced.label).toBe('typed here ');
    expect(synced.notes).toBe('from another window');
  });

  it('keeps the typed text when the saved value is its normalized form', () => {
    const form = edit(formFromData(HOST_FIELDS, stored), { label: 'web ' });
    const saved = { ...stored, label: 'web' };
    const synced = syncForm(HOST_FIELDS, form, stored, saved);
    expect(synced.label).toBe('web ');
    expect(buildData(HOST_FIELDS, saved, validateForm(HOST_FIELDS, synced)).changed).toBe(false);
  });

  it('requires a group name', () => {
    const form = formFromData(GROUP_FIELDS, {});
    expect(validateForm(GROUP_FIELDS, form).errors).toEqual([{ path: 'name', reason: 'required' }]);
  });
});
