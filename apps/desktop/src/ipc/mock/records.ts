// Records of the mock's personal vault: hosts and groups with the query,
// save, delete, restore, move and duplicate rules of the plan (sections
// 5.4-5.6) and docs/data-model.md (sections 7, 8, 18.1).
//
// Owner: W4.

import { IpcError } from '../errors';
import type {
  FieldError,
  GroupView,
  HostSecretPath,
  HostView,
  InheritedValues,
  MovePosition,
  RecordDeleteResult,
  RecordMoveInput,
  RecordSaveInput,
  RecordType,
  RecordView,
  RecordsQuery,
} from '../types';
import { isUuid } from './ids';
import { compareOrderKeys, evenlySpacedKeys, orderKeyBetween } from './orderKey';
import { HOST_SECRET_PATHS, checkSecret, isObject, validateGroupData, validateHostData, type DataObject } from './validate';

export interface StoredRecord {
  record_id: string;
  type: RecordType;
  data: DataObject;
  /** Secret members by path (never part of `data`). */
  secrets: Partial<Record<HostSecretPath, string>>;
  revision: number;
  /** Creation time (the UUIDv7 time of `record_id`). */
  createdMs: number;
  modifiedMs: number;
  /** Hybrid logical clock stand-in: increases with every write. */
  hlc: number;
}

export interface RecordStoreDeps {
  vaultId: string;
  newId: () => { id: string; ms: number };
  now: () => number;
}

/** Undo window of a removal (plan, decision 14). */
export const UNDO_WINDOW_MS = 60_000;
/** Maximum nesting depth of groups. */
const MAX_DEPTH = 32;
/** Maximum serialized payload (storage-format 5.3). */
const MAX_PAYLOAD_BYTES = 262_143;

const KNOWN_HOST_MEMBERS = new Set([
  'address',
  'label',
  'group_id',
  'tags',
  'notes',
  'color',
  'order_key',
  'backspace',
  'charset',
  'terminal_theme',
  'record_history',
  'record_output',
  'primary_protocol',
  'ssh',
  'telnet',
  'os',
  'import_key',
]);
const KNOWN_GROUP_MEMBERS = new Set(['name', 'parent_id', 'color', 'notes', 'order_key', 'ssh', 'import_key']);
const MANAGED_MEMBERS = ['order_key', 'import_key', 'os'] as const;
const INHERITED_FIELDS = ['port', 'username', 'identity_id', 'key_id', 'jump_chain'] as const;

const clone = <T>(value: T): T => structuredClone(value);
const iso = (ms: number) => new Date(ms).toISOString();
const lower = (text: string) => text.toLowerCase();

function validationError(errors: FieldError[]): IpcError {
  return new IpcError('validation_failed', { errors });
}

function stringMember(data: DataObject, key: string): string | undefined {
  const value = data[key];
  return typeof value === 'string' ? value : undefined;
}

function sshOf(data: DataObject): DataObject {
  return isObject(data.ssh) ? data.ssh : {};
}

export class RecordStore {
  private readonly records = new Map<string, StoredRecord>();
  private readonly undo = new Map<string, { record: StoredRecord; expires: number }>();
  private hlcCounter = 0;
  private readonly deps: RecordStoreDeps;

  constructor(deps: RecordStoreDeps) {
    this.deps = deps;
  }

  get vaultId(): string {
    return this.deps.vaultId;
  }

  /** Drops the undo buffer (on lock). */
  clearUndo(): void {
    this.undo.clear();
  }

  /** Inserts a record without validation (fixtures). */
  insert(type: RecordType, data: DataObject, secrets: StoredRecord['secrets'] = {}): StoredRecord {
    const { id, ms } = this.deps.newId();
    const record: StoredRecord = {
      record_id: id,
      type,
      data: clone(data),
      secrets: { ...secrets },
      revision: 1,
      createdMs: ms,
      modifiedMs: ms,
      hlc: this.nextHlc(),
    };
    record.data.order_key = this.keyAtEnd(type, this.containerOf(record), record.record_id);
    this.records.set(id, record);
    return record;
  }

  get(recordId: string): StoredRecord | undefined {
    return this.records.get(recordId);
  }

  all(): StoredRecord[] {
    return [...this.records.values()];
  }

  private nextHlc(): number {
    this.hlcCounter += 1;
    return this.hlcCounter;
  }

  private ofType(type: RecordType): StoredRecord[] {
    return this.all().filter((record) => record.type === type);
  }

  private isGroup(id: unknown): id is string {
    return typeof id === 'string' && this.records.get(id)?.type === 'group';
  }

  // --- Tree -------------------------------------------------------------

  /** Resolved parent of every group: a missing parent is top level; in a cycle the group with the greatest HLC is top level. */
  private resolvedParents(): Map<string, string | null> {
    const parents = new Map<string, string | null>();
    for (const group of this.ofType('group')) {
      const parent = group.data.parent_id;
      parents.set(group.record_id, this.isGroup(parent) && parent !== group.record_id ? parent : null);
    }
    const done = new Set<string>();
    for (const start of parents.keys()) {
      const path: string[] = [];
      const onPath = new Set<string>();
      let current: string | null = start;
      while (current !== null && !done.has(current)) {
        if (onPath.has(current)) {
          const cycle = path.slice(path.indexOf(current));
          const top = cycle.reduce((best, id) =>
            (this.records.get(id)?.hlc ?? 0) > (this.records.get(best)?.hlc ?? 0) ? id : best,
          );
          parents.set(top, null);
          break;
        }
        onPath.add(current);
        path.push(current);
        current = parents.get(current) ?? null;
      }
      for (const id of path) {
        done.add(id);
      }
    }
    return parents;
  }

  /** Group ids from `groupId` up to the top level (at most 32). */
  private chainUp(groupId: string | null, parents: Map<string, string | null>): string[] {
    const chain: string[] = [];
    let current = groupId;
    while (current !== null && chain.length < MAX_DEPTH && !chain.includes(current)) {
      chain.push(current);
      current = parents.get(current) ?? null;
    }
    return chain;
  }

  private hostGroup(host: StoredRecord): string | null {
    const groupId = host.data.group_id;
    return this.isGroup(groupId) ? groupId : null;
  }

  private inheritedFrom(groupId: string | null, parents: Map<string, string | null>): InheritedValues {
    const inherited: InheritedValues = {};
    const chain = this.chainUp(groupId, parents);
    for (const field of INHERITED_FIELDS) {
      for (const id of chain) {
        const group = this.records.get(id);
        const value = group === undefined ? undefined : sshOf(group.data)[field];
        if (value !== undefined && value !== null) {
          (inherited as Record<string, unknown>)[field] = { value: clone(value), from_group_id: id };
          break;
        }
      }
    }
    return inherited;
  }

  /** Every group in the subtree of `groupId` (itself included). */
  private subtree(groupId: string, parents: Map<string, string | null>): Set<string> {
    const result = new Set<string>([groupId]);
    let added = true;
    while (added) {
      added = false;
      for (const [id, parent] of parents) {
        if (parent !== null && result.has(parent) && !result.has(id)) {
          result.add(id);
          added = true;
        }
      }
    }
    return result;
  }

  private depthOf(groupId: string | null, parents: Map<string, string | null>): number {
    return this.chainUp(groupId, parents).length;
  }

  /** Height of the subtree of `groupId` (1 for a group without subgroups). */
  private heightOf(groupId: string, parents: Map<string, string | null>): number {
    let height = 1;
    for (const [id, parent] of parents) {
      if (parent === groupId) {
        height = Math.max(height, 1 + this.heightOf(id, parents));
      }
    }
    return height;
  }

  // --- Views ------------------------------------------------------------

  view(record: StoredRecord, parents = this.resolvedParents()): RecordView {
    const base = {
      vault_id: this.deps.vaultId,
      record_id: record.record_id,
      revision: record.revision,
      created_at: iso(record.createdMs),
      modified_at: iso(record.modifiedMs),
    };
    if (record.type === 'host') {
      const groupId = this.hostGroup(record);
      const chain = this.chainUp(groupId, parents);
      const view: HostView = {
        ...base,
        type: 'host',
        data: clone(record.data) as HostView['data'],
        inherited: this.inheritedFrom(groupId, parents),
        path: chain.reverse(),
        secrets_set: HOST_SECRET_PATHS.filter((path) => record.secrets[path] !== undefined),
      };
      return view;
    }
    const parent = parents.get(record.record_id) ?? null;
    const subtree = this.subtree(record.record_id, parents);
    const view: GroupView = {
      ...base,
      type: 'group',
      data: clone(record.data) as GroupView['data'],
      inherited: this.inheritedFrom(parent, parents),
      path: this.chainUp(parent, parents).reverse(),
      host_count: this.ofType('host').filter((host) => {
        const groupId = this.hostGroup(host);
        return groupId !== null && subtree.has(groupId);
      }).length,
      child_count:
        this.ofType('group').filter((group) => parents.get(group.record_id) === record.record_id).length +
        this.ofType('host').filter((host) => host.data.group_id === record.record_id).length,
    };
    return view;
  }

  // --- Query ------------------------------------------------------------

  query(query: RecordsQuery): RecordView[] {
    if (query.vault_id !== undefined && query.vault_id !== this.deps.vaultId) {
      throw new IpcError('not_found');
    }
    const parents = this.resolvedParents();
    const scope = query.scope ?? { kind: 'all' };
    const text = lower((query.text ?? '').trim());
    const tags = (query.tags ?? []).map(lower);
    const types = new Set(query.types);

    const inScope = (record: StoredRecord): boolean => {
      if (scope.kind === 'all') {
        return true;
      }
      const container =
        record.type === 'group' ? (parents.get(record.record_id) ?? null) : this.hostGroup(record);
      if (scope.kind === 'top_level') {
        return container === null;
      }
      if (record.type === 'group') {
        return container === scope.group_id;
      }
      return record.data.group_id === scope.group_id;
    };

    const matchesText = (record: StoredRecord): boolean => {
      if (text === '') {
        return true;
      }
      if (record.type === 'group') {
        return lower(stringMember(record.data, 'name') ?? '').includes(text);
      }
      const candidates = [stringMember(record.data, 'label'), stringMember(record.data, 'address')];
      for (const tag of (record.data.tags as unknown[] | undefined) ?? []) {
        candidates.push(typeof tag === 'string' ? tag : undefined);
      }
      const ownUsername = sshOf(record.data).username;
      const username =
        typeof ownUsername === 'string'
          ? ownUsername
          : this.inheritedFrom(this.hostGroup(record), parents).username?.value;
      candidates.push(username);
      return candidates.some((candidate) => candidate !== undefined && lower(candidate).includes(text));
    };

    const matchesTags = (record: StoredRecord): boolean => {
      if (tags.length === 0) {
        return true;
      }
      if (record.type === 'group') {
        return false;
      }
      const own = ((record.data.tags as unknown[] | undefined) ?? [])
        .filter((tag): tag is string => typeof tag === 'string')
        .map(lower);
      return tags.every((tag) => own.includes(tag));
    };

    const selected = this.all().filter(
      (record) => types.has(record.type) && inScope(record) && matchesText(record) && matchesTags(record),
    );
    const sort = query.sort ?? 'manual';
    const compare = (a: StoredRecord, b: StoredRecord): number => {
      const byId = a.record_id < b.record_id ? -1 : a.record_id > b.record_id ? 1 : 0;
      switch (sort) {
        case 'name': {
          const title = (record: StoredRecord) =>
            lower(
              record.type === 'group'
                ? (stringMember(record.data, 'name') ?? '')
                : (stringMember(record.data, 'label') ?? stringMember(record.data, 'address') ?? ''),
            );
          const ta = title(a);
          const tb = title(b);
          return ta < tb ? -1 : ta > tb ? 1 : byId;
        }
        case 'created':
          return b.createdMs - a.createdMs || -byId;
        case 'modified':
          return b.hlc - a.hlc || -byId;
        case 'manual':
        default: {
          const ka = stringMember(a.data, 'order_key');
          const kb = stringMember(b.data, 'order_key');
          if (ka !== undefined && kb !== undefined) {
            return compareOrderKeys(ka, kb) || byId;
          }
          if (ka !== undefined) {
            return -1;
          }
          if (kb !== undefined) {
            return 1;
          }
          return byId;
        }
      }
    };
    const groups = selected.filter((record) => record.type === 'group').sort(compare);
    const hosts = selected.filter((record) => record.type === 'host').sort(compare);
    return [...groups, ...hosts].map((record) => this.view(record, parents));
  }

  getView(recordId: string): RecordView {
    const record = this.records.get(recordId);
    if (record === undefined) {
      throw new IpcError('not_found');
    }
    return this.view(record);
  }

  // --- Order keys -------------------------------------------------------

  private containerOf(record: StoredRecord): string | null {
    const value = record.type === 'group' ? record.data.parent_id : record.data.group_id;
    return typeof value === 'string' ? value : null;
  }

  /** Records of a type in a container, in manual order. */
  private containerItems(type: RecordType, container: string | null, exclude: ReadonlySet<string> = new Set()): StoredRecord[] {
    return this.ofType(type)
      .filter((record) => this.containerOf(record) === container && !exclude.has(record.record_id))
      .sort((a, b) => {
        const ka = stringMember(a.data, 'order_key');
        const kb = stringMember(b.data, 'order_key');
        if (ka !== undefined && kb !== undefined) {
          return compareOrderKeys(ka, kb) || (a.record_id < b.record_id ? -1 : 1);
        }
        if (ka !== undefined) {
          return -1;
        }
        if (kb !== undefined) {
          return 1;
        }
        return a.record_id < b.record_id ? -1 : 1;
      });
  }

  /** Rewrites the keys of a container with evenly spaced keys (one write per item). */
  private rebalance(type: RecordType, container: string | null, exclude: ReadonlySet<string>): void {
    const items = this.containerItems(type, container, exclude);
    const keys = evenlySpacedKeys(items.length);
    items.forEach((record, index) => {
      record.data.order_key = keys[index];
      record.revision += 1;
      record.hlc = this.nextHlc();
    });
  }

  private keyAtEnd(type: RecordType, container: string | null, selfId: string): string {
    const items = this.containerItems(type, container, new Set([selfId]));
    const last = items.length === 0 ? null : (stringMember(items[items.length - 1].data, 'order_key') ?? null);
    const key = orderKeyBetween(last, null);
    if (key !== null) {
      return key;
    }
    this.rebalance(type, container, new Set([selfId]));
    return this.keyAtEnd(type, container, selfId);
  }

  /** `count` consecutive keys between two neighbours of a container. */
  private keysBetween(low: string | null, high: string | null, count: number): string[] | null {
    const keys: string[] = [];
    let previous = low;
    for (let index = 0; index < count; index += 1) {
      const key = orderKeyBetween(previous, high);
      if (key === null) {
        return null;
      }
      keys.push(key);
      previous = key;
    }
    return keys;
  }

  // --- Writes -----------------------------------------------------------

  private touch(record: StoredRecord): void {
    record.revision += 1;
    record.hlc = this.nextHlc();
    record.modifiedMs = Math.max(this.deps.now(), record.modifiedMs);
  }

  save(input: RecordSaveInput): RecordView {
    if (input.vault_id !== this.deps.vaultId) {
      throw new IpcError('not_found');
    }
    const existing = input.record_id === null || input.record_id === undefined ? undefined : this.records.get(input.record_id);
    if (input.record_id !== null && input.record_id !== undefined && (existing === undefined || existing.type !== input.type)) {
      throw new IpcError('not_found');
    }
    if (!isObject(input.data)) {
      throw validationError([{ field: 'data', reason: 'invalid' }]);
    }
    const selfId = existing?.record_id ?? null;
    const parents = this.resolvedParents();
    const { data, errors } =
      input.type === 'host' ? validateHostData(input.data, selfId) : validateGroupData(input.data);

    // Secrets (host only).
    const secretChanges: [HostSecretPath, string | null][] = [];
    const rawSecrets: unknown = (input as { secrets?: unknown }).secrets;
    if (rawSecrets !== undefined && rawSecrets !== null) {
      if (!isObject(rawSecrets)) {
        errors.push({ field: 'secrets', reason: 'invalid' });
      } else {
        for (const [path, value] of Object.entries(rawSecrets)) {
          if (input.type !== 'host' || !(HOST_SECRET_PATHS as readonly string[]).includes(path)) {
            errors.push({ field: path, reason: 'not_allowed' });
          } else if (value === null) {
            secretChanges.push([path as HostSecretPath, null]);
          } else if (typeof value !== 'string') {
            errors.push({ field: path, reason: 'invalid' });
          } else {
            const reason = checkSecret(value, 4096);
            if (reason !== null) {
              errors.push({ field: path, reason });
            } else {
              secretChanges.push([path as HostSecretPath, value]);
            }
          }
        }
      }
    }

    // Group tree rules.
    if (input.type === 'group' && typeof data.parent_id === 'string' && errors.every((error) => error.field !== 'parent_id')) {
      const parentId = data.parent_id;
      if (selfId !== null && (parentId === selfId || this.subtree(selfId, parents).has(parentId))) {
        errors.push({ field: 'parent_id', reason: 'cycle' });
      } else if (this.isGroup(parentId)) {
        const height = selfId === null ? 1 : this.heightOf(selfId, parents);
        if (this.depthOf(parentId, parents) + height > MAX_DEPTH) {
          errors.push({ field: 'parent_id', reason: 'too_deep' });
        }
      }
    }
    if (errors.length > 0) {
      throw validationError(errors);
    }

    // Members the mock does not know: from the stored data and the input (input wins).
    const known = input.type === 'host' ? KNOWN_HOST_MEMBERS : KNOWN_GROUP_MEMBERS;
    const merged: DataObject = {};
    if (existing !== undefined) {
      for (const [key, value] of Object.entries(existing.data)) {
        if (!known.has(key)) {
          merged[key] = clone(value);
        }
      }
    }
    for (const [key, value] of Object.entries(data)) {
      if (!(MANAGED_MEMBERS as readonly string[]).includes(key)) {
        merged[key] = value;
      }
    }
    if (existing !== undefined) {
      for (const key of MANAGED_MEMBERS) {
        if (existing.data[key] !== undefined) {
          merged[key] = clone(existing.data[key]);
        }
      }
    }

    const record: StoredRecord =
      existing !== undefined
        ? { ...existing, data: merged, secrets: { ...existing.secrets } }
        : (() => {
            const { id, ms } = this.deps.newId();
            return {
              record_id: id,
              type: input.type,
              data: merged,
              secrets: {},
              revision: 0,
              createdMs: ms,
              modifiedMs: ms,
              hlc: 0,
            };
          })();
    for (const [path, value] of secretChanges) {
      if (value === null) {
        delete record.secrets[path];
      } else {
        record.secrets[path] = value;
      }
    }
    const containerChanged = existing === undefined || this.containerOf(existing) !== this.containerOf(record);
    if (containerChanged || record.data.order_key === undefined) {
      record.data.order_key = this.keyAtEnd(record.type, this.containerOf(record), record.record_id);
    }
    if (JSON.stringify({ data: record.data, secrets: record.secrets }).length > MAX_PAYLOAD_BYTES) {
      throw new IpcError('too_large');
    }
    this.touch(record);
    this.records.set(record.record_id, record);
    return this.view(record);
  }

  remove(recordIds: readonly string[]): RecordDeleteResult {
    for (const id of recordIds) {
      if (!this.records.has(id)) {
        throw new IpcError('not_found');
      }
    }
    const removing = new Set(recordIds);
    const moved: string[] = [];
    const now = this.deps.now();
    for (const id of recordIds) {
      const record = this.records.get(id);
      if (record === undefined) {
        continue;
      }
      this.undo.set(id, { record: clone(record), expires: now + UNDO_WINDOW_MS });
      if (record.type === 'group') {
        const parents = this.resolvedParents();
        const newParent = parents.get(id) ?? null;
        const children = this.all().filter(
          (child) =>
            child.record_id !== id &&
            !removing.has(child.record_id) &&
            (child.type === 'group' ? child.data.parent_id === id : child.data.group_id === id),
        );
        for (const child of children) {
          const key = child.type === 'group' ? 'parent_id' : 'group_id';
          if (newParent === null) {
            delete child.data[key];
          } else {
            child.data[key] = newParent;
          }
          child.data.order_key = this.keyAtEnd(child.type, newParent, child.record_id);
          this.touch(child);
          moved.push(child.record_id);
        }
      }
      this.records.delete(id);
    }
    return { deleted: [...recordIds], moved };
  }

  restore(recordIds: readonly string[]): RecordView[] {
    const now = this.deps.now();
    for (const id of recordIds) {
      const entry = this.undo.get(id);
      if (entry === undefined || entry.expires <= now || this.records.has(id)) {
        throw new IpcError('not_found');
      }
    }
    const restored: StoredRecord[] = [];
    for (const id of recordIds) {
      const entry = this.undo.get(id);
      if (entry === undefined) {
        continue;
      }
      this.undo.delete(id);
      const record = clone(entry.record);
      this.touch(record);
      this.records.set(id, record);
      restored.push(record);
    }
    const parents = this.resolvedParents();
    return restored.map((record) => this.view(record, parents));
  }

  move(input: RecordMoveInput): RecordView[] {
    if (input.vault_id !== this.deps.vaultId) {
      throw new IpcError('not_found');
    }
    const records = input.record_ids.map((id) => {
      const record = this.records.get(id);
      if (record === undefined) {
        throw new IpcError('not_found');
      }
      return record;
    });
    const target = input.target_group_id ?? null;
    if (target !== null && !this.isGroup(target)) {
      throw validationError([{ field: 'target_group_id', reason: 'missing_ref' }]);
    }
    const parents = this.resolvedParents();
    for (const record of records) {
      if (record.type !== 'group' || target === null) {
        continue;
      }
      if (this.subtree(record.record_id, parents).has(target)) {
        throw validationError([{ field: 'target_group_id', reason: 'cycle' }]);
      }
      if (this.depthOf(target, parents) + this.heightOf(record.record_id, parents) > MAX_DEPTH) {
        throw validationError([{ field: 'target_group_id', reason: 'too_deep' }]);
      }
    }
    const position: MovePosition = input.position ?? { kind: 'end' };
    const movingIds = new Set(input.record_ids);
    let anchor: StoredRecord | undefined;
    if (position.kind !== 'end') {
      anchor = this.records.get(position.record_id);
      if (anchor === undefined || movingIds.has(anchor.record_id) || this.containerOf(anchor) !== target) {
        throw validationError([{ field: 'position', reason: 'invalid' }]);
      }
    }
    for (const type of ['group', 'host'] as const) {
      const moving = records.filter((record) => record.type === type);
      if (moving.length === 0) {
        continue;
      }
      const assignKeys = (): string[] | null => {
        const items = this.containerItems(type, target, movingIds);
        if (anchor === undefined || anchor.type !== type) {
          const last = items.length === 0 ? null : (stringMember(items[items.length - 1].data, 'order_key') ?? null);
          return this.keysBetween(last, null, moving.length);
        }
        const index = items.findIndex((item) => item.record_id === anchor?.record_id);
        const before = position.kind === 'before';
        const lowItem = before ? items[index - 1] : items[index];
        const highItem = before ? items[index] : items[index + 1];
        const low = lowItem === undefined ? null : (stringMember(lowItem.data, 'order_key') ?? null);
        const high = highItem === undefined ? null : (stringMember(highItem.data, 'order_key') ?? null);
        return this.keysBetween(low, high, moving.length);
      };
      let keys = assignKeys();
      if (keys === null) {
        this.rebalance(type, target, movingIds);
        keys = assignKeys();
      }
      moving.forEach((record, index) => {
        const key = record.type === 'group' ? 'parent_id' : 'group_id';
        if (target === null) {
          delete record.data[key];
        } else {
          record.data[key] = target;
        }
        record.data.order_key = keys?.[index] ?? this.keyAtEnd(type, target, record.record_id);
        this.touch(record);
      });
    }
    const after = this.resolvedParents();
    return records.map((record) => this.view(record, after));
  }

  duplicate(recordId: string): RecordView {
    const original = this.records.get(recordId);
    if (original === undefined) {
      throw new IpcError('not_found');
    }
    if (original.type !== 'host') {
      throw validationError([{ field: 'type', reason: 'invalid' }]);
    }
    const { id, ms } = this.deps.newId();
    const data = clone(original.data);
    delete data.import_key;
    const title = stringMember(original.data, 'label') ?? stringMember(original.data, 'address') ?? '';
    data.label = `${title} Copy`;
    const container = this.containerOf(original);
    const items = this.containerItems('host', container);
    const index = items.findIndex((item) => item.record_id === recordId);
    const next = items[index + 1];
    let key = orderKeyBetween(
      stringMember(original.data, 'order_key') ?? null,
      next === undefined ? null : (stringMember(next.data, 'order_key') ?? null),
    );
    const copy: StoredRecord = {
      record_id: id,
      type: 'host',
      data,
      secrets: { ...original.secrets },
      revision: 0,
      createdMs: ms,
      modifiedMs: ms,
      hlc: 0,
    };
    if (key === null) {
      this.rebalance('host', container, new Set());
      key = this.keyAtEnd('host', container, id);
    }
    copy.data.order_key = key;
    this.touch(copy);
    this.records.set(id, copy);
    return this.view(copy);
  }

  secret(recordId: string, path: string): string {
    const record = this.records.get(recordId);
    if (record === undefined) {
      throw new IpcError('not_found');
    }
    if (record.type !== 'host' || !(HOST_SECRET_PATHS as readonly string[]).includes(path)) {
      throw validationError([{ field: 'path', reason: 'not_allowed' }]);
    }
    const value = record.secrets[path as HostSecretPath];
    if (value === undefined) {
      throw new IpcError('not_found');
    }
    return value;
  }

  /** Ids are UUID text (input check shared by several commands). */
  static checkIds(ids: readonly unknown[]): void {
    if (!ids.every(isUuid)) {
      throw new IpcError('not_found');
    }
  }
}
