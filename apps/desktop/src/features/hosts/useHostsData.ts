// Records behind the Hosts screen (stage plan 6.6): the visible list (the
// current folder, or the whole vault while filtering), every group (for
// breadcrumbs, pickers and inherited values) and every host (for tags and the
// panel). Reloaded on `records:changed` for the vault, debounced 50 ms.

import { useEffect, useMemo, useState } from 'react';
import { isIpcError, recordsQuery, subscribe } from '../../ipc';
import type { GroupView, HostView, RecordView, SortMode, Uuid } from '../../ipc';
import { buildListQuery } from './format';

export const RELOAD_DEBOUNCE_MS = 50;

export interface HostsDataInput {
  vaultId: Uuid;
  folderId: Uuid | null;
  text: string;
  tags: readonly string[];
  sort: SortMode;
}

export interface HostsData {
  /** Every query has answered at least once. */
  ready: boolean;
  /** Visible groups, in the query's order. */
  groups: readonly GroupView[];
  /** Visible hosts, in the query's order. */
  hosts: readonly HostView[];
  /** Every group of the vault, sorted by name. */
  allGroups: readonly GroupView[];
  /** Every host of the vault. */
  allHosts: readonly HostView[];
  groupsById: ReadonlyMap<Uuid, GroupView>;
  hostsById: ReadonlyMap<Uuid, HostView>;
}

interface VaultRecords {
  vaultId: Uuid;
  groups: GroupView[];
  hosts: HostView[];
}

interface ListRecords {
  items: RecordView[];
}

function reportError(reason: unknown): void {
  // A lock unmounts the screen; queries that were already running fail with
  // `locked`. Other failures keep the last data; the next change retries.
  if (!isIpcError(reason, 'locked') && import.meta.env.DEV) {
    console.warn('records_query failed', isIpcError(reason) ? reason.code : 'unknown');
  }
}

export function useHostsData({ vaultId, folderId, text, tags, sort }: HostsDataInput): HostsData {
  const [reloadKey, setReloadKey] = useState(0);
  const [vaultRecords, setVaultRecords] = useState<VaultRecords | null>(null);
  const [list, setList] = useState<ListRecords | null>(null);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void subscribe('records:changed', (event) => {
      if (event.vault_id !== vaultId) {
        return;
      }
      clearTimeout(timer);
      timer = setTimeout(() => setReloadKey((key) => key + 1), RELOAD_DEBOUNCE_MS);
    }).then((stop) => {
      if (disposed) {
        stop();
      } else {
        unlisten = stop;
      }
    });
    return () => {
      disposed = true;
      clearTimeout(timer);
      unlisten?.();
    };
  }, [vaultId]);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      recordsQuery({ vault_id: vaultId, types: ['group'], scope: { kind: 'all' }, sort: 'name' }),
      recordsQuery({ vault_id: vaultId, types: ['host'], scope: { kind: 'all' }, sort: 'manual' }),
    ]).then(
      ([groups, hosts]) => {
        if (!cancelled) {
          setVaultRecords({
            vaultId,
            groups: groups.filter((record): record is GroupView => record.type === 'group'),
            hosts: hosts.filter((record): record is HostView => record.type === 'host'),
          });
        }
      },
      reportError,
    );
    return () => {
      cancelled = true;
    };
  }, [vaultId, reloadKey]);

  // Tags are compared by content, not by array identity.
  const tagsKey = tags.join('\u0000');
  const query = useMemo(
    () => buildListQuery({ vaultId, folderId, text, tags: tagsKey === '' ? [] : tagsKey.split('\u0000'), sort }),
    [vaultId, folderId, text, tagsKey, sort],
  );

  useEffect(() => {
    let cancelled = false;
    recordsQuery(query).then((items) => {
      if (!cancelled) {
        setList({ items });
      }
    }, reportError);
    return () => {
      cancelled = true;
    };
  }, [query, reloadKey]);

  return useMemo(() => {
    const current = vaultRecords !== null && vaultRecords.vaultId === vaultId ? vaultRecords : null;
    const allGroups = current?.groups ?? [];
    const allHosts = current?.hosts ?? [];
    // The previous folder's list stays visible until the new one arrives.
    const items = list?.items ?? [];
    return {
      ready: current !== null && list !== null,
      groups: items.filter((record): record is GroupView => record.type === 'group'),
      hosts: items.filter((record): record is HostView => record.type === 'host'),
      allGroups,
      allHosts,
      groupsById: new Map(allGroups.map((group) => [group.record_id, group])),
      hostsById: new Map(allHosts.map((host) => [host.record_id, host])),
    };
  }, [vaultRecords, list, vaultId]);
}
