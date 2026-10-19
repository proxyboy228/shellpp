// Data behind the host picker (UI 6, 4.7): Recent connections and the hosts
// of the personal vault (their views carry inherited usernames for search).
// Reloaded on `records:changed` (class 7 connection log, class 2 hosts) and
// whenever `refreshKey` changes (the new tab page becomes active). Nothing is
// kept while the window is locked: the terminal area stays mounted under the
// unlock screen, so decrypted host data is dropped on lock.
//
// Owner: S7.

import { useEffect, useMemo, useState } from 'react';
import { useLockStatus, useVaults } from '../../app/store';
import {
  isIpcError,
  recentConnections,
  recordsQuery,
  subscribe,
  type HostView,
  type RecentConnection,
  type Uuid,
} from '../../ipc';

/** Record class codes (docs/data-model.md, section 2.2). */
const CLASS_HOSTS = 0x02;
const CLASS_CONNECTION_LOG = 0x07;
const RELOAD_DEBOUNCE_MS = 50;

export interface PickerData {
  /** Both lists answered at least once (and the window is unlocked). */
  ready: boolean;
  vaultId: Uuid | null;
  recent: readonly RecentConnection[];
  hosts: readonly HostView[];
}

function reportError(reason: unknown): void {
  if (!isIpcError(reason, 'locked') && import.meta.env.DEV) {
    console.warn('host picker query failed', isIpcError(reason) ? reason.code : 'unknown');
  }
}

export function usePickerData(refreshKey: number): PickerData {
  const lock = useLockStatus();
  const vaults = useVaults();
  const unlocked = lock?.state === 'unlocked';
  const vaultId = vaults.find((vault) => vault.kind === 'personal')?.vault_id ?? null;
  const [reloadKey, setReloadKey] = useState(0);
  const [recent, setRecent] = useState<readonly RecentConnection[] | null>(null);
  const [records, setRecords] = useState<{ vaultId: Uuid; hosts: HostView[] } | null>(null);

  // Decrypted data does not stay in memory while locked.
  const [wasUnlocked, setWasUnlocked] = useState(unlocked);
  if (wasUnlocked !== unlocked) {
    setWasUnlocked(unlocked);
    if (!unlocked) {
      setRecent(null);
      setRecords(null);
    }
  }

  useEffect(() => {
    if (!unlocked || vaultId === null) {
      return undefined;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void subscribe('records:changed', (event) => {
      if (event.vault_id !== vaultId || !event.classes.some((code) => code === CLASS_HOSTS || code === CLASS_CONNECTION_LOG)) {
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
  }, [unlocked, vaultId]);

  useEffect(() => {
    if (!unlocked || vaultId === null) {
      return undefined;
    }
    let cancelled = false;
    Promise.all([
      recentConnections(),
      recordsQuery({ vault_id: vaultId, types: ['host'], scope: { kind: 'all' }, sort: 'name' }),
    ]).then(([rows, list]) => {
      if (cancelled) {
        return;
      }
      setRecent(rows);
      setRecords({
        vaultId,
        hosts: list.filter((record): record is HostView => record.type === 'host'),
      });
    }, reportError);
    return () => {
      cancelled = true;
    };
  }, [unlocked, vaultId, reloadKey, refreshKey]);

  return useMemo(() => {
    const current = unlocked && records !== null && records.vaultId === vaultId ? records : null;
    return {
      ready: current !== null && recent !== null,
      vaultId,
      recent: unlocked ? (recent ?? []) : [],
      hosts: current?.hosts ?? [],
    };
  }, [unlocked, records, recent, vaultId]);
}
