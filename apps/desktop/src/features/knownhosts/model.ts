// Known Hosts list rules of the screen (UI 10.1; stage 2 plan 6.3): titles,
// search and sorting.
//
// Owner: S7.

import type { KnownHostView } from '../../ipc';
import { t } from '../../i18n';

export type KnownHostsSort = 'name' | 'created' | 'modified';
export type KnownHostsLayout = 'grid' | 'list';

export const KNOWN_HOSTS_SORTS: readonly KnownHostsSort[] = ['name', 'created', 'modified'];
export const KNOWN_HOSTS_LAYOUTS: readonly KnownHostsLayout[] = ['grid', 'list'];

/** Card title: the OpenSSH display address, or "Hashed entry". */
export function knownHostTitle(entry: KnownHostView): string {
  return entry.hashed || entry.display_address === undefined ? t('known_hosts.hashed_entry') : entry.display_address;
}

/** Case-insensitive search over host, display address, key type, fingerprint and comment. */
export function filterKnownHosts(entries: readonly KnownHostView[], text: string): KnownHostView[] {
  const query = text.trim().toLowerCase();
  if (query === '') {
    return [...entries];
  }
  return entries.filter((entry) =>
    [entry.host, entry.display_address, entry.key_type, entry.fingerprint, entry.comment].some(
      (value) => value !== undefined && value.toLowerCase().includes(query),
    ),
  );
}

/** Case-insensitive code point order of display addresses (as the core sorts them). */
function byAddress(a: KnownHostView, b: KnownHostView): number {
  const x = (a.display_address ?? '').toLowerCase();
  const y = (b.display_address ?? '').toLowerCase();
  return x < y ? -1 : x > y ? 1 : 0;
}

const byId = (a: KnownHostView, b: KnownHostView) => (a.record_id < b.record_id ? -1 : a.record_id > b.record_id ? 1 : 0);

/**
 * Name: display address (case-insensitive), hashed entries last by creation.
 * Created and Modified: newest first.
 */
export function sortKnownHosts(entries: readonly KnownHostView[], mode: KnownHostsSort): KnownHostView[] {
  const list = [...entries];
  switch (mode) {
    case 'created':
      return list.sort((a, b) => b.created_at.localeCompare(a.created_at) || byId(b, a));
    case 'modified':
      return list.sort((a, b) => b.modified_at.localeCompare(a.modified_at) || byId(b, a));
    case 'name':
    default:
      return list.sort((a, b) => {
        if (a.hashed !== b.hashed) {
          return a.hashed ? 1 : -1;
        }
        if (a.hashed) {
          return a.created_at.localeCompare(b.created_at) || byId(a, b);
        }
        return byAddress(a, b) || byId(a, b);
      });
  }
}
