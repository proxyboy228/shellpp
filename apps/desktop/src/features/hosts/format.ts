// Display text of host and group cards (UI 2.3) and the queries behind the
// Hosts screen (stage plan 5.4, 6.6).

import type { GroupView, HostView, QueryScope, RecordsQuery, SortMode, Uuid } from '../../ipc';

/** Card title: the label, or the address when the label is empty. */
export function hostTitle(host: HostView): string {
  const label = host.data.label?.trim() ?? '';
  return label !== '' ? label : host.data.address;
}

/** The host's own username, else the inherited one. */
export function effectiveUsername(host: HostView): string | undefined {
  const own = host.data.ssh?.username;
  if (typeof own === 'string' && own !== '') {
    return own;
  }
  return host.inherited.username?.value;
}

/**
 * Card subtitle: protocol(s), the effective username, then the group name
 * (while filtering, when results come from the whole vault) or the tags.
 */
export function hostSubtitle(host: HostView, filtering: boolean, groupsById: ReadonlyMap<Uuid, GroupView>): string {
  const parts: string[] = ['SSH'];
  if (host.data.telnet !== undefined && host.data.telnet !== null) {
    parts.push('Telnet');
  }
  const username = effectiveUsername(host);
  if (username !== undefined) {
    parts.push(username);
  }
  if (filtering) {
    const groupId = host.data.group_id;
    const group = groupId === undefined ? undefined : groupsById.get(groupId);
    if (group !== undefined) {
      parts.push(group.data.name);
    }
  } else if (host.data.tags !== undefined && host.data.tags.length > 0) {
    parts.push(host.data.tags.join(', '));
  }
  return parts.join(', ');
}

/** Text or tags set: the sections show results from the whole vault. */
export function isFiltering(text: string, tags: readonly string[]): boolean {
  return text.trim() !== '' || tags.length > 0;
}

export interface ListQueryInput {
  vaultId: Uuid;
  folderId: Uuid | null;
  text: string;
  tags: readonly string[];
  sort: SortMode;
}

/** The query of the visible list: the current folder, or the whole vault while filtering. */
export function buildListQuery({ vaultId, folderId, text, tags, sort }: ListQueryInput): RecordsQuery {
  const trimmed = text.trim();
  const filtering = isFiltering(trimmed, tags);
  let scope: QueryScope;
  if (filtering) {
    scope = { kind: 'all' };
  } else if (folderId === null) {
    scope = { kind: 'top_level' };
  } else {
    scope = { kind: 'group', group_id: folderId };
  }
  const query: RecordsQuery = { vault_id: vaultId, types: ['group', 'host'], scope, sort };
  if (trimmed !== '') {
    query.text = trimmed;
  }
  if (tags.length > 0) {
    query.tags = [...tags];
  }
  return query;
}

/** Every distinct tag of the hosts (case-insensitive, first spelling), sorted. */
export function collectTags(hosts: readonly HostView[]): string[] {
  const seen = new Map<string, string>();
  for (const host of hosts) {
    for (const tag of host.data.tags ?? []) {
      const key = tag.toLowerCase();
      if (!seen.has(key)) {
        seen.set(key, tag);
      }
    }
  }
  return [...seen.values()].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
}

/** The group and its whole subtree (by the core-resolved paths). */
export function subtreeIds(groupId: Uuid, groups: readonly GroupView[]): Set<Uuid> {
  const ids = new Set<Uuid>([groupId]);
  for (const group of groups) {
    if (group.path.includes(groupId)) {
      ids.add(group.record_id);
    }
  }
  return ids;
}

/** `prod / web` style label of a group for menus and pickers. */
export function groupPathLabel(group: GroupView, groupsById: ReadonlyMap<Uuid, GroupView>): string {
  const names = group.path.map((id) => groupsById.get(id)?.data.name).filter((name): name is string => name !== undefined);
  return [...names, group.data.name].join(' / ');
}

/** Groups with their path labels, in tree order (sorted by the path label). */
export function sortedByPath(
  groups: readonly GroupView[],
  groupsById: ReadonlyMap<Uuid, GroupView>,
): { group: GroupView; label: string }[] {
  const segments = (group: GroupView) =>
    [...group.path.map((id) => groupsById.get(id)?.data.name ?? ''), group.data.name].map((name) => name.toLowerCase());
  return groups
    .map((group) => ({ group, label: groupPathLabel(group, groupsById), key: segments(group) }))
    .sort((a, b) => {
      for (let index = 0; index < Math.min(a.key.length, b.key.length); index += 1) {
        const order = (a.key[index] ?? '').localeCompare(b.key[index] ?? '');
        if (order !== 0) {
          return order;
        }
      }
      return a.key.length - b.key.length;
    })
    .map(({ group, label }) => ({ group, label }));
}
