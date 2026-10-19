// Items of the host picker (UI 6): Recent connections while the search is
// empty; while typing, open tabs of this window first, then hosts matched by
// label, address, username and tags (like the Hosts search).
//
// Owner: S7.

import type { HostView, RecentConnection, SessionTarget } from '../../ipc';
import { effectiveUsername, hostTitle } from '../hosts/format';
import type { TabSummary } from '../terminal';

export type PickerItem =
  | { kind: 'recent'; key: string; target: SessionTarget; row: RecentConnection }
  | { kind: 'tab'; key: string; tab: TabSummary }
  | { kind: 'host'; key: string; target: SessionTarget; host: HostView };

/** Hosts shown at most while typing. */
export const MAX_HOST_RESULTS = 50;

/** What a Recent connections row connects to: its saved host, else the address it used. */
export function recentTarget(row: RecentConnection): SessionTarget {
  if (row.host !== undefined) {
    return { kind: 'host', vault_id: row.host.vault_id, record_id: row.host.record_id };
  }
  const target: SessionTarget = { kind: 'quick', address: row.address };
  if (row.port !== 22) {
    target.port = row.port;
  }
  if (row.username !== undefined && row.username !== '') {
    target.username = row.username;
  }
  return target;
}

export function hostTarget(host: HostView): SessionTarget {
  return { kind: 'host', vault_id: host.vault_id, record_id: host.record_id };
}

/** Case-insensitive match on label, address, the effective username and tags. */
export function hostMatches(host: HostView, query: string): boolean {
  const text = query.trim().toLowerCase();
  if (text === '') {
    return true;
  }
  const candidates = [host.data.label, host.data.address, effectiveUsername(host), ...(host.data.tags ?? [])];
  return candidates.some((candidate) => typeof candidate === 'string' && candidate.toLowerCase().includes(text));
}

/** Session tabs whose title or address match (new-tab pages are not listed). */
export function tabMatches(tab: TabSummary, query: string): boolean {
  const text = query.trim().toLowerCase();
  if (tab.kind !== 'session') {
    return false;
  }
  return [tab.title, tab.address].some((value) => value !== undefined && value.toLowerCase().includes(text));
}

export interface PickerInput {
  query: string;
  includeTabs: boolean;
  recent: readonly RecentConnection[];
  tabs: readonly TabSummary[];
  hosts: readonly HostView[];
}

/** The rows of the picker in display (and keyboard) order. */
export function pickerItems({ query, includeTabs, recent, tabs, hosts }: PickerInput): PickerItem[] {
  if (query.trim() === '') {
    return recent.map((row, index) => ({
      kind: 'recent',
      key: row.host !== undefined ? `recent-host-${row.host.record_id}` : `recent-${row.address}-${row.port}-${index}`,
      target: recentTarget(row),
      row,
    }));
  }
  const items: PickerItem[] = [];
  if (includeTabs) {
    for (const tab of tabs) {
      if (tabMatches(tab, query)) {
        items.push({ kind: 'tab', key: `tab-${tab.id}`, tab });
      }
    }
  }
  const matched = hosts
    .filter((host) => hostMatches(host, query))
    .sort((a, b) => hostTitle(a).localeCompare(hostTitle(b), undefined, { sensitivity: 'base' }))
    .slice(0, MAX_HOST_RESULTS);
  for (const host of matched) {
    items.push({ kind: 'host', key: `host-${host.record_id}`, target: hostTarget(host), host });
  }
  return items;
}
