// Shared state of the stage-2 mock modules (stage 2 plan, section 6.6):
// `sessions.ts` (S6: sessions, local terminal, history) and `connect.ts`
// (S7: known hosts, keys, recent connections). Both get one context from
// `createMockBackend`, so a TOFU accept in a mock session shows up in the
// mock Known Hosts list and a mock connection in Recent connections.
//
// Everything is in memory and fake: keys and fingerprints are made-up
// strings, never real key material.
//
// Owner: lead (contract). S6 and S7 may add fields in their own modules'
// state objects, not here.

import type {
  CommandInput,
  CommandName,
  CommandOutput,
  EventMap,
  EventName,
  KnownHostSource,
  LockState,
  RecordRef,
  SettingsValues,
} from '../types';
import type { RecordStore } from './records';

/** A command handler of the mock. */
export type MockHandler<C extends CommandName> = (
  input: CommandInput<C>,
) => CommandOutput<C> | Promise<CommandOutput<C>>;

export interface MockKnownHost {
  record_id: string;
  host: string;
  port?: number;
  key_type: string;
  public_key: string;
  revoked: boolean;
  source: KnownHostSource;
  comment?: string;
  created_at: string;
  modified_at: string;
}

export interface MockKey {
  record_id: string;
  label: string;
  algorithm: string;
  fingerprint?: string;
  encrypted: boolean;
  /** The saved passphrase (mock only); null when not saved. */
  passphrase: string | null;
  created_at: string;
  modified_at: string;
}

export interface MockHistoryEntry {
  record_id: string;
  host?: RecordRef;
  command: string;
  executed_at: string;
}

export interface MockConnection {
  host?: RecordRef;
  title: string;
  address: string;
  port: number;
  username?: string;
  last_at: string;
}

export interface Stage2MockContext {
  readonly vaultId: string;
  readonly windowLabel: string;
  now(): number;
  /** A new UUIDv7-shaped id. */
  newId(): string;
  readonly records: RecordStore;
  lockState(): LockState;
  settings(): SettingsValues;
  /** Emits a window event (no `v`): `sessions:changed`, `window:close-requested`. */
  emit<E extends EventName>(event: E, payload: EventMap[E]): void;
  /** Emits `records:changed` for the personal vault with these classes. */
  recordsChanged(recordIds: readonly string[], classes: readonly number[]): void;
  readonly knownHosts: MockKnownHost[];
  readonly keys: MockKey[];
  readonly history: MockHistoryEntry[];
  readonly connections: MockConnection[];
  /** Handlers of channels created with `createChannel`, by channel id. */
  readonly channels: Map<number, (message: unknown) => void>;
}

/** Record class codes (docs/data-model.md, section 2.2). */
export const CLASS_HOSTS = 0x02;
export const CLASS_KEYS = 0x03;
export const CLASS_HISTORY = 0x06;
export const CLASS_CONNECTION_LOG = 0x07;
export const CLASS_KNOWN_HOSTS = 0x08;
