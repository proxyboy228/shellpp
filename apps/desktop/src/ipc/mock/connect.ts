// Mock known hosts, keys and recent connections (stage 2 plan, section 6.6).
//
// - `known_hosts_list` from `ctx.knownHosts` with the view rules of plan 4.4
//   and 6.3: display address (`host` for port 22, `[host]:port` otherwise,
//   absent for hashed `|1|...` entries), sorted by display address with
//   hashed entries last, `host_ids` = saved hosts whose (address, effective
//   port) the entry matches (plain names and `*`/`?` patterns; hashed entries
//   are not matched by the mock).
// - `known_host_remove` (`not_found` for unknown ids).
// - `keys_list` / `key_import` with a fake OpenSSH parser: the text must
//   start with `-----BEGIN OPENSSH PRIVATE KEY-----`; optional lines
//   `mock-algorithm:<name>`, `mock-comment:<text>` and
//   `mock-encrypted:<passphrase>` describe the key (the passphrase must equal
//   that value, so tests generate it at run time). Errors follow the core
//   (plan 4.4, keys).
// - `recent_connections` from `ctx.connections`: newest first, one row per
//   host link (or address and port), at most 10, current host titles and
//   group paths.
//
// Fingerprints are made-up strings from a non-cryptographic hash (known hosts
// use the sessions mock's `mockFingerprint`, so the Known Hosts screen shows
// what the host key dialogs showed): the mock never handles real key material.
//
// Owner: S7. The handler set and the control interface are the contract.

import { IpcError } from '../errors';
import type {
  FieldError,
  HostView,
  KeyImportInput,
  KeyImportResult,
  KeyView,
  KnownHostAddInput,
  KnownHostView,
  RecentConnection,
  RecordView,
} from '../types';
import {
  CLASS_CONNECTION_LOG,
  CLASS_KEYS,
  CLASS_KNOWN_HOSTS,
  type MockConnection,
  type MockHandler,
  type MockKey,
  type MockKnownHost,
  type Stage2MockContext,
} from './context';
import { mockFingerprint } from './sessions';

export type ConnectCommand = 'known_hosts_list' | 'known_host_remove' | 'keys_list' | 'key_import' | 'recent_connections';

export type ConnectHandlers = { [C in ConnectCommand]: MockHandler<C> };

/** Test helpers (exposed as `backend.control.connect`). */
export interface ConnectMockControl {
  /** Adds a known host entry as an import would (also the e2e mock action `known_host_add`). */
  addKnownHost(input: KnownHostAddInput): string;
  /** Records a finished connection as the connection log would (Recent connections). */
  addConnection(connection: Omit<MockConnection, 'last_at'> & { last_at?: string }): void;
  /** The key records, without the mock passphrase. */
  keys(): KeyView[];
}

export interface ConnectMock {
  handlers: ConnectHandlers;
  control: ConnectMockControl;
}

/** The mock's personal vault name (`createMockBackend`). */
const VAULT_NAME = 'Personal';
const KEY_HEADER = '-----BEGIN OPENSSH PRIVATE KEY-----';
const MAX_PRIVATE_KEY_BYTES = 32_768;
const MAX_LABEL = 256;
const MAX_RECENT = 10;
const SUPPORTED_ALGORITHMS = new Set([
  'ssh-ed25519',
  'ecdsa-sha2-nistp256',
  'ecdsa-sha2-nistp384',
  'ecdsa-sha2-nistp521',
  'ssh-rsa',
]);

/** A deterministic made-up `SHA256:` fingerprint (FNV-1a, not a hash of key material). */
export function fakeFingerprint(text: string): string {
  const bytes: number[] = [];
  let hash = 0x811c9dc5;
  for (let round = 0; bytes.length < 32; round += 1) {
    for (let index = 0; index < text.length; index += 1) {
      hash ^= text.charCodeAt(index) + round;
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
    bytes.push(hash & 0xff, (hash >>> 8) & 0xff, (hash >>> 16) & 0xff, (hash >>> 24) & 0xff);
  }
  const binary = String.fromCharCode(...bytes.slice(0, 32));
  return `SHA256:${btoa(binary).replace(/=+$/, '')}`;
}

const isHashed = (host: string) => host.startsWith('|1|');

/** `host` for port 22 (or none), `[host]:port` otherwise; undefined for hashed entries. */
export function knownHostDisplayAddress(host: string, port: number | undefined): string | undefined {
  if (isHashed(host)) {
    return undefined;
  }
  return port === undefined || port === 22 ? host : `[${host}]:${port}`;
}

function wildcardMatch(pattern: string, name: string): boolean {
  const source = pattern
    .toLowerCase()
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*/g, '.*')
    .replace(/\?/g, '.');
  return new RegExp(`^${source}$`).test(name.toLowerCase());
}

function entryMatches(entry: MockKnownHost, address: string, port: number): boolean {
  if (isHashed(entry.host)) {
    return false;
  }
  if ((entry.port ?? 22) !== port) {
    return false;
  }
  if (entry.host.includes('*') || entry.host.includes('?')) {
    return wildcardMatch(entry.host, address);
  }
  return entry.host.toLowerCase() === address.toLowerCase();
}

function hostViews(ctx: Stage2MockContext): HostView[] {
  return ctx.records
    .query({ vault_id: ctx.vaultId, types: ['host'], scope: { kind: 'all' }, sort: 'manual' })
    .filter((record: RecordView): record is HostView => record.type === 'host');
}

function effectivePort(host: HostView): number {
  return host.data.ssh?.port ?? host.inherited.port?.value ?? 22;
}

function hostTitle(host: HostView): string {
  const label = host.data.label?.trim() ?? '';
  return label === '' ? host.data.address : label;
}

function knownHostView(ctx: Stage2MockContext, entry: MockKnownHost, hosts: readonly HostView[]): KnownHostView {
  const view: KnownHostView = {
    vault_id: ctx.vaultId,
    record_id: entry.record_id,
    created_at: entry.created_at,
    modified_at: entry.modified_at,
    host: entry.host,
    hashed: isHashed(entry.host),
    key_type: entry.key_type,
    // The same made-up fingerprint the mock sessions show in the host key dialogs.
    fingerprint: mockFingerprint(entry.public_key),
    revoked: entry.revoked,
    source: entry.source,
    host_ids: hosts
      .filter((host) => entryMatches(entry, host.data.address, effectivePort(host)))
      .map((host) => host.record_id),
  };
  if (entry.port !== undefined) {
    view.port = entry.port;
  }
  const display = knownHostDisplayAddress(entry.host, entry.port);
  if (display !== undefined) {
    view.display_address = display;
  }
  if (entry.comment !== undefined) {
    view.comment = entry.comment;
  }
  return view;
}

function keyView(ctx: Stage2MockContext, key: MockKey): KeyView {
  const view: KeyView = {
    vault_id: ctx.vaultId,
    record_id: key.record_id,
    created_at: key.created_at,
    modified_at: key.modified_at,
    label: key.label,
    algorithm: key.algorithm,
    encrypted: key.encrypted,
    passphrase_saved: key.passphrase !== null,
  };
  if (key.fingerprint !== undefined) {
    view.fingerprint = key.fingerprint;
  }
  return view;
}

/** Case-insensitive code point order (the core compares lowercased text). */
const byText = (a: string, b: string) => {
  const x = a.toLowerCase();
  const y = b.toLowerCase();
  return x < y ? -1 : x > y ? 1 : 0;
};

interface ParsedKey {
  algorithm: string;
  comment?: string;
  /** The passphrase that opens the key, or null for an unencrypted key. */
  passphrase: string | null;
  fingerprint: string;
}

function refused(errors: FieldError[]): IpcError {
  return new IpcError('validation_failed', { errors });
}

/** The fake parser of plan 6.6. Throws the core's field errors. */
function parseKey(text: string): ParsedKey {
  if (!text.startsWith(KEY_HEADER)) {
    throw refused([{ field: 'private_key', reason: 'invalid' }]);
  }
  const value = (name: string) => {
    const line = text.split('\n').find((candidate) => candidate.startsWith(`mock-${name}:`));
    return line === undefined ? undefined : line.slice(name.length + 6);
  };
  const algorithm = value('algorithm') ?? 'ssh-ed25519';
  if (!SUPPORTED_ALGORITHMS.has(algorithm)) {
    throw refused([{ field: 'private_key', reason: 'not_allowed' }]);
  }
  const comment = value('comment');
  return {
    algorithm,
    ...(comment === undefined || comment === '' ? {} : { comment }),
    passphrase: value('encrypted') ?? null,
    fingerprint: fakeFingerprint(text),
  };
}

export function createConnectMock(ctx: Stage2MockContext): ConnectMock {
  const iso = () => new Date(ctx.now()).toISOString();

  const keyImport = (input: KeyImportInput): KeyImportResult => {
    if (input.vault_id !== ctx.vaultId) {
      throw new IpcError('not_found');
    }
    const text = input.private_key.replace(/\r\n/g, '\n').trim();
    if (new TextEncoder().encode(text).length > MAX_PRIVATE_KEY_BYTES) {
      throw refused([{ field: 'private_key', reason: 'too_long' }]);
    }
    const parsed = parseKey(text);
    const passphrase = input.passphrase ?? '';
    if (parsed.passphrase === null && passphrase !== '') {
      throw refused([{ field: 'passphrase', reason: 'not_allowed' }]);
    }
    if (parsed.passphrase !== null) {
      if (passphrase === '') {
        throw refused([{ field: 'passphrase', reason: 'required' }]);
      }
      if (passphrase !== parsed.passphrase) {
        throw refused([{ field: 'passphrase', reason: 'invalid' }]);
      }
    }
    const label = (input.label ?? '').trim();
    if (label.length > MAX_LABEL) {
      throw refused([{ field: 'label', reason: 'too_long' }]);
    }
    const save = input.save_passphrase === true && parsed.passphrase !== null;
    const existing = ctx.keys.find((key) => key.fingerprint === parsed.fingerprint);
    if (existing !== undefined) {
      if (save && existing.passphrase === null) {
        existing.passphrase = passphrase;
        existing.modified_at = iso();
        ctx.recordsChanged([existing.record_id], [CLASS_KEYS]);
      }
      return { key: keyView(ctx, existing), created: false };
    }
    const at = iso();
    const key: MockKey = {
      record_id: ctx.newId(),
      label: label !== '' ? label : (parsed.comment ?? parsed.algorithm),
      algorithm: parsed.algorithm,
      fingerprint: parsed.fingerprint,
      encrypted: parsed.passphrase !== null,
      passphrase: save ? passphrase : null,
      created_at: at,
      modified_at: at,
    };
    ctx.keys.push(key);
    ctx.recordsChanged([key.record_id], [CLASS_KEYS]);
    return { key: keyView(ctx, key), created: true };
  };

  const recent = (): RecentConnection[] => {
    const hosts = new Map(hostViews(ctx).map((host) => [host.record_id, host]));
    const groupName = (id: string) => {
      try {
        const view = ctx.records.getView(id);
        return view.type === 'group' ? view.data.name : undefined;
      } catch {
        return undefined;
      }
    };
    const seen = new Set<string>();
    const rows: RecentConnection[] = [];
    const ordered = [...ctx.connections].sort((a, b) => (a.last_at < b.last_at ? 1 : a.last_at > b.last_at ? -1 : 0));
    for (const connection of ordered) {
      const key =
        connection.host !== undefined ? `host:${connection.host.record_id}` : `quick:${connection.address.toLowerCase()}:${connection.port}`;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      const host = connection.host === undefined ? undefined : hosts.get(connection.host.record_id);
      const path = [VAULT_NAME];
      if (host !== undefined) {
        for (const id of host.path) {
          const name = groupName(id);
          if (name !== undefined) {
            path.push(name);
          }
        }
      }
      const row: RecentConnection = {
        title: host !== undefined ? hostTitle(host) : connection.title || connection.address,
        address: connection.address,
        port: connection.port,
        path,
        last_at: connection.last_at,
      };
      if (connection.host !== undefined && host !== undefined) {
        row.host = { ...connection.host };
      }
      const username = connection.username;
      if (username !== undefined) {
        row.username = username;
      }
      if (host?.data.os !== undefined) {
        row.os = host.data.os;
      }
      rows.push(row);
      if (rows.length >= MAX_RECENT) {
        break;
      }
    }
    return rows;
  };

  return {
    handlers: {
      known_hosts_list: () => {
        const hosts = hostViews(ctx);
        const views = ctx.knownHosts.map((entry) => knownHostView(ctx, entry, hosts));
        const plain = views.filter((view) => !view.hashed).sort((a, b) => byText(a.display_address ?? '', b.display_address ?? ''));
        const hashed = views.filter((view) => view.hashed).sort((a, b) => (a.created_at < b.created_at ? -1 : 1));
        return [...plain, ...hashed];
      },
      known_host_remove: (input) => {
        if (input.vault_id !== ctx.vaultId || input.record_ids.some((id) => !ctx.knownHosts.some((entry) => entry.record_id === id))) {
          throw new IpcError('not_found');
        }
        for (const id of input.record_ids) {
          const index = ctx.knownHosts.findIndex((entry) => entry.record_id === id);
          ctx.knownHosts.splice(index, 1);
        }
        ctx.recordsChanged(input.record_ids, [CLASS_KNOWN_HOSTS]);
        return null;
      },
      keys_list: () => [...ctx.keys].sort((a, b) => byText(a.label, b.label)).map((key) => keyView(ctx, key)),
      key_import: keyImport,
      recent_connections: recent,
    },
    control: {
      addKnownHost: (input) => {
        const id = ctx.newId();
        const at = new Date(ctx.now()).toISOString();
        ctx.knownHosts.push({
          record_id: id,
          host: input.host,
          port: input.port,
          key_type: input.key_type,
          public_key: input.public_key,
          revoked: input.revoked ?? false,
          source: 'import',
          comment: input.comment,
          created_at: at,
          modified_at: at,
        });
        ctx.recordsChanged([id], [CLASS_KNOWN_HOSTS]);
        return id;
      },
      addConnection: (connection) => {
        ctx.connections.push({ ...connection, last_at: connection.last_at ?? iso() });
        ctx.recordsChanged([ctx.newId()], [CLASS_CONNECTION_LOG]);
      },
      keys: () => ctx.keys.map((key) => keyView(ctx, key)),
    },
  };
}
