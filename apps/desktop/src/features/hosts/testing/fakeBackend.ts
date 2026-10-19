// A small in-memory backend for the Hosts screen and panel tests. It follows
// the record rules of the stage plan (5.4-5.6) closely enough for UI tests:
// scopes, text and tag search, sort modes, inherited values, paths, counts,
// save with required fields and port ranges, delete with undo, move,
// duplicate and secret reveal. It shares no code with the panel validators.
// Test data uses documentation-only addresses.

import type { Backend } from '../../../ipc';
import { IpcError } from '../../../ipc';
import type {
  IpcChannel,
  CommandInput,
  CommandName,
  CommandOutput,
  EventMap,
  EventName,
  FieldError,
  GroupData,
  GroupView,
  HostData,
  HostView,
  InheritedValues,
  RecordType,
  RecordView,
  SettingsValues,
  Uuid,
} from '../../../ipc';

export const FAKE_VAULT_ID = '01990000-0000-7000-8000-00000000aaaa';

interface StoredRecord {
  type: RecordType;
  record_id: Uuid;
  revision: number;
  created: number;
  modified: number;
  order: number;
  data: Record<string, unknown>;
  secrets: Record<string, string>;
}

export interface FakeCall {
  command: CommandName;
  input: unknown;
}

export interface FakeBackend extends Backend {
  readonly calls: FakeCall[];
  readonly records: Map<Uuid, StoredRecord>;
  clipboard: string | null;
  settings: SettingsValues;
  addGroup: (data: Partial<GroupData> & { name: string }) => Uuid;
  addHost: (data: Partial<HostData> & { address: string }, secrets?: Record<string, string>) => Uuid;
  emit: <E extends EventName>(event: E, payload: EventMap[E]) => void;
  /** Makes the next call of `command` fail with this error. */
  failNext: (command: CommandName, error: IpcError) => void;
  callsOf: (command: CommandName) => unknown[];
  host: (id: Uuid) => HostData;
  group: (id: Uuid) => GroupData;
}

let idCounter = 0;

function nextId(): Uuid {
  idCounter += 1;
  return `01990000-0000-7000-8000-${idCounter.toString(16).padStart(12, '0')}`;
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function createFakeBackend(initialSettings: SettingsValues = {}): FakeBackend {
  const records = new Map<Uuid, StoredRecord>();
  const listeners = new Map<EventName, Set<(payload: unknown) => void>>();
  const calls: FakeCall[] = [];
  const failures = new Map<CommandName, IpcError>();
  const undo = new Map<Uuid, StoredRecord>();
  let clock = 0;
  let version = 0;

  const tick = () => {
    clock += 1;
    return clock;
  };

  const groupOf = (id: unknown): StoredRecord | undefined => {
    if (typeof id !== 'string') {
      return undefined;
    }
    const record = records.get(id);
    return record?.type === 'group' ? record : undefined;
  };

  /** Ancestors of a group, top level first (cycle-safe). */
  const ancestors = (group: StoredRecord): Uuid[] => {
    const path: Uuid[] = [];
    const seen = new Set<Uuid>([group.record_id]);
    let parent = groupOf(group.data.parent_id);
    while (parent !== undefined && !seen.has(parent.record_id) && path.length < 32) {
      path.unshift(parent.record_id);
      seen.add(parent.record_id);
      parent = groupOf(parent.data.parent_id);
    }
    return path;
  };

  const containerOf = (record: StoredRecord): Uuid | null => {
    const parent = groupOf(record.type === 'host' ? record.data.group_id : record.data.parent_id);
    return parent?.record_id ?? null;
  };

  const inheritedFrom = (startGroup: StoredRecord | undefined): InheritedValues => {
    const result: InheritedValues = {};
    if (startGroup === undefined) {
      return result;
    }
    const chain = [startGroup.record_id, ...ancestors(startGroup).reverse()];
    for (const id of chain) {
      const ssh = (records.get(id)?.data.ssh ?? {}) as { port?: number; username?: string };
      if (result.port === undefined && typeof ssh.port === 'number') {
        result.port = { value: ssh.port, from_group_id: id };
      }
      if (result.username === undefined && typeof ssh.username === 'string') {
        result.username = { value: ssh.username, from_group_id: id };
      }
    }
    return result;
  };

  const effectiveUsername = (record: StoredRecord): string | undefined => {
    const own = (record.data.ssh as { username?: string } | undefined)?.username;
    return own ?? inheritedFrom(groupOf(record.data.group_id)).username?.value;
  };

  const view = (record: StoredRecord): RecordView => {
    const base = {
      vault_id: FAKE_VAULT_ID,
      record_id: record.record_id,
      revision: record.revision,
      created_at: new Date(Date.UTC(2026, 0, 1, 0, 0, record.created)).toISOString(),
      modified_at: new Date(Date.UTC(2026, 0, 1, 0, 0, record.modified)).toISOString(),
    };
    if (record.type === 'group') {
      const subtree = [...records.values()].filter(
        (item) => item.type === 'group' && (item.record_id === record.record_id || ancestors(item).includes(record.record_id)),
      );
      const subtreeIds = new Set(subtree.map((item) => item.record_id));
      const hosts = [...records.values()].filter((item) => item.type === 'host' && subtreeIds.has(String(item.data.group_id)));
      const children = [...records.values()].filter((item) => item.record_id !== record.record_id && containerOf(item) === record.record_id);
      const groupView: GroupView = {
        ...base,
        type: 'group',
        data: clone(record.data) as GroupData,
        inherited: inheritedFrom(groupOf(record.data.parent_id)),
        path: ancestors(record),
        host_count: hosts.length,
        child_count: children.length,
      };
      return groupView;
    }
    const group = groupOf(record.data.group_id);
    const hostView: HostView = {
      ...base,
      type: 'host',
      data: clone(record.data) as HostData,
      inherited: inheritedFrom(group),
      path: group === undefined ? [] : [...ancestors(group), group.record_id],
      secrets_set: Object.keys(record.secrets).filter((path): path is 'ssh.password' => path === 'ssh.password'),
    };
    return hostView;
  };

  const emit = <E extends EventName>(event: E, payload: EventMap[E]) => {
    for (const handler of listeners.get(event) ?? []) {
      handler(payload);
    }
  };

  const changed = (ids: Uuid[]) => {
    version += 1;
    emit('records:changed', { v: version, vault_id: FAKE_VAULT_ID, record_ids: ids, classes: [2] });
  };

  const siblings = (type: RecordType, container: Uuid | null) =>
    [...records.values()]
      .filter((item) => item.type === type && containerOf(item) === container)
      .sort((a, b) => a.order - b.order || a.record_id.localeCompare(b.record_id));

  const endOrder = (type: RecordType, container: Uuid | null) =>
    siblings(type, container).reduce((max, item) => Math.max(max, item.order), 0) + 1;

  const renumber = (list: StoredRecord[]) => {
    list.forEach((item, index) => {
      item.order = index + 1;
    });
  };

  const insert = (type: RecordType, data: Record<string, unknown>, secrets: Record<string, string> = {}) => {
    const record: StoredRecord = {
      type,
      record_id: nextId(),
      revision: 1,
      created: tick(),
      modified: clock,
      order: 0,
      data: clone(data),
      secrets: { ...secrets },
    };
    records.set(record.record_id, record);
    record.order = endOrder(type, containerOf(record));
    return record;
  };

  const validate = (type: RecordType, data: Record<string, unknown>): FieldError[] => {
    const errors: FieldError[] = [];
    if (type === 'host' && (typeof data.address !== 'string' || data.address.trim() === '')) {
      errors.push({ field: 'address', reason: 'required' });
    }
    if (type === 'group' && (typeof data.name !== 'string' || data.name.trim() === '')) {
      errors.push({ field: 'name', reason: 'required' });
    }
    const port = (data.ssh as { port?: unknown } | undefined)?.port;
    if (port !== undefined && (typeof port !== 'number' || port < 1 || port > 65535)) {
      errors.push({ field: 'ssh.port', reason: 'out_of_range' });
    }
    return errors;
  };

  const matchesText = (record: StoredRecord, text: string) => {
    const needle = text.toLowerCase();
    if (record.type === 'group') {
      return String(record.data.name).toLowerCase().includes(needle);
    }
    const fields = [record.data.label, record.data.address, effectiveUsername(record), ...((record.data.tags as string[]) ?? [])];
    return fields.some((field) => typeof field === 'string' && field.toLowerCase().includes(needle));
  };

  const handlers: { [C in CommandName]?: (input: CommandInput<C>) => CommandOutput<C> } = {
    records_query: (query) => {
      const types = new Set(query.types);
      const scope = query.scope ?? { kind: 'all' };
      const text = query.text?.trim() ?? '';
      const tags = (query.tags ?? []).map((tag) => tag.toLowerCase());
      let items = [...records.values()].filter((record) => types.has(record.type));
      if (scope.kind === 'top_level') {
        items = items.filter((record) => containerOf(record) === null);
      } else if (scope.kind === 'group') {
        items = items.filter((record) => containerOf(record) === scope.group_id);
      }
      if (text !== '') {
        items = items.filter((record) => matchesText(record, text));
      }
      if (tags.length > 0) {
        items = items.filter((record) => {
          const own = ((record.data.tags as string[] | undefined) ?? []).map((tag) => tag.toLowerCase());
          return record.type === 'host' && tags.every((tag) => own.includes(tag));
        });
      }
      const sort = query.sort ?? 'manual';
      const title = (record: StoredRecord) =>
        String(record.type === 'group' ? record.data.name : record.data.label || record.data.address).toLowerCase();
      const compare = (a: StoredRecord, b: StoredRecord) => {
        switch (sort) {
          case 'name':
            return title(a).localeCompare(title(b)) || a.record_id.localeCompare(b.record_id);
          case 'created':
            return b.created - a.created;
          case 'modified':
            return b.modified - a.modified;
          default:
            return a.order - b.order || a.record_id.localeCompare(b.record_id);
        }
      };
      const groups = items.filter((record) => record.type === 'group').sort(compare);
      const hosts = items.filter((record) => record.type === 'host').sort(compare);
      return [...groups, ...hosts].map(view);
    },
    record_get: (ref) => {
      const record = records.get(ref.record_id);
      if (record === undefined) {
        throw new IpcError('not_found');
      }
      return view(record);
    },
    record_save: (input) => {
      const data = clone(input.data) as Record<string, unknown>;
      delete data.order_key;
      const errors = validate(input.type, data);
      if (errors.length > 0) {
        throw new IpcError('validation_failed', { errors });
      }
      const secrets = input.type === 'host' ? (input.secrets ?? {}) : {};
      if (input.record_id === null || input.record_id === undefined) {
        const created = insert(input.type, data);
        for (const [path, value] of Object.entries(secrets)) {
          if (typeof value === 'string') {
            created.secrets[path] = value;
          }
        }
        changed([created.record_id]);
        return view(created);
      }
      const record = records.get(input.record_id);
      if (record === undefined || record.type !== input.type) {
        throw new IpcError('not_found');
      }
      const before = containerOf(record);
      record.data = { ...record.data, ...data };
      for (const key of Object.keys(record.data)) {
        if (!(key in data)) {
          delete record.data[key];
        }
      }
      for (const [path, value] of Object.entries(secrets)) {
        if (value === null) {
          delete record.secrets[path];
        } else if (typeof value === 'string') {
          record.secrets[path] = value;
        }
      }
      record.revision += 1;
      record.modified = tick();
      if (containerOf(record) !== before) {
        record.order = endOrder(record.type, containerOf(record)) + 1;
      }
      changed([record.record_id]);
      return view(record);
    },
    record_delete: (input) => {
      const deleted: Uuid[] = [];
      const moved: Uuid[] = [];
      for (const id of input.record_ids) {
        const record = records.get(id);
        if (record === undefined) {
          throw new IpcError('not_found');
        }
        if (record.type === 'group') {
          const parent = containerOf(record);
          for (const child of [...records.values()]) {
            if (child.record_id !== id && containerOf(child) === id) {
              if (child.type === 'host') {
                child.data.group_id = parent ?? undefined;
              } else {
                child.data.parent_id = parent ?? undefined;
              }
              if (parent === null) {
                delete child.data[child.type === 'host' ? 'group_id' : 'parent_id'];
              }
              child.order = endOrder(child.type, parent) + 1;
              moved.push(child.record_id);
            }
          }
        }
        undo.set(id, clone(record));
        records.delete(id);
        deleted.push(id);
      }
      changed([...deleted, ...moved]);
      return { deleted, moved };
    },
    record_restore: (input) => {
      const restored: RecordView[] = [];
      for (const id of input.record_ids) {
        const record = undo.get(id);
        if (record === undefined) {
          throw new IpcError('not_found');
        }
        undo.delete(id);
        record.revision += 1;
        record.modified = tick();
        records.set(id, record);
        restored.push(view(record));
      }
      changed([...input.record_ids]);
      return restored;
    },
    record_move: (input) => {
      const target = input.target_group_id ?? null;
      const moving = input.record_ids.map((id) => {
        const record = records.get(id);
        if (record === undefined) {
          throw new IpcError('not_found');
        }
        return record;
      });
      for (const record of moving) {
        if (target === null) {
          delete record.data[record.type === 'host' ? 'group_id' : 'parent_id'];
        } else {
          record.data[record.type === 'host' ? 'group_id' : 'parent_id'] = target;
        }
        record.revision += 1;
        record.modified = tick();
      }
      const type = moving[0]?.type ?? 'host';
      const position = input.position ?? { kind: 'end' };
      const list = siblings(type, target).filter((item) => !moving.includes(item));
      let index = list.length;
      if (position.kind !== 'end') {
        const anchor = list.findIndex((item) => item.record_id === position.record_id);
        if (anchor < 0) {
          throw new IpcError('validation_failed', { errors: [{ field: 'position', reason: 'invalid' }] });
        }
        index = position.kind === 'before' ? anchor : anchor + 1;
      }
      list.splice(index, 0, ...moving);
      renumber(list);
      changed([...input.record_ids]);
      return moving.map(view);
    },
    record_duplicate: (ref) => {
      const original = records.get(ref.record_id);
      if (original === undefined || original.type !== 'host') {
        throw new IpcError('not_found');
      }
      const label = String(original.data.label || original.data.address);
      const copy = insert('host', { ...original.data, label: `${label} Copy` }, original.secrets);
      const list = siblings('host', containerOf(original)).filter((item) => item !== copy);
      list.splice(list.indexOf(original) + 1, 0, copy);
      renumber(list);
      changed([copy.record_id]);
      return view(copy);
    },
    secret_reveal: (field) => {
      const value = records.get(field.record_id)?.secrets[field.path];
      if (value === undefined) {
        throw new IpcError('not_found');
      }
      return { value };
    },
    clipboard_write_text: (input) => {
      backend.clipboard = input.text;
      return null;
    },
    settings_get: () => backend.settings,
    settings_set: (input) => {
      backend.settings = { ...backend.settings, ...(input.values as SettingsValues) };
      version += 1;
      emit('settings:changed', { v: version, keys: Object.keys(input.values ?? {}) });
      return backend.settings;
    },
  };

  const backend: FakeBackend = {
    kind: 'mock',
    calls,
    records,
    clipboard: null,
    settings: { ...initialSettings },
    invoke<C extends CommandName>(command: C, input: CommandInput<C>): Promise<CommandOutput<C>> {
      calls.push({ command, input: clone(input ?? null) });
      const failure = failures.get(command);
      if (failure !== undefined) {
        failures.delete(command);
        return Promise.reject(failure);
      }
      const handler = handlers[command] as ((value: CommandInput<C>) => CommandOutput<C>) | undefined;
      if (handler === undefined) {
        return Promise.reject(new IpcError('internal'));
      }
      try {
        return Promise.resolve(handler(input));
      } catch (error) {
        return Promise.reject(error);
      }
    },
    listen<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void) {
      const set = listeners.get(event) ?? new Set();
      const wrapped = handler as (payload: unknown) => void;
      set.add(wrapped);
      listeners.set(event, set);
      return Promise.resolve(() => {
        set.delete(wrapped);
      });
    },
    // Stage 2 contract (lead): the hosts screen opens no channels.
    createChannel: <T,>() => ({ __ipcChannel: true as const, close: () => undefined }) as IpcChannel<T> & { close(): void },
    window: {
      label: 'w-1',
      minimize: () => Promise.resolve(),
      toggleMaximize: () => Promise.resolve(),
      isMaximized: () => Promise.resolve(false),
      close: () => Promise.resolve(),
      startDragging: () => Promise.resolve(),
      setFullscreen: () => Promise.resolve(),
      onResized: () => Promise.resolve(() => undefined),
    },
    addGroup: (data) => insert('group', data).record_id,
    addHost: (data, secrets) => insert('host', data, secrets).record_id,
    emit,
    failNext: (command, error) => {
      failures.set(command, error);
    },
    callsOf: (command) => calls.filter((call) => call.command === command).map((call) => call.input),
    host: (id) => records.get(id)?.data as HostData,
    group: (id) => records.get(id)?.data as GroupData,
  };
  return backend;
}

/** A vault view for the screen's `vault` prop. */
export const FAKE_VAULT = {
  vault_id: FAKE_VAULT_ID,
  kind: 'personal',
  status: 'active',
  name: 'Personal',
  color: 'graphite',
} as const;
