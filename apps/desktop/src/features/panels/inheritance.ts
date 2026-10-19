// Inherited SSH values for the "from group X" placeholders (UI 3.1, 3.4;
// docs/data-model.md 8.2-8.3). Computed from the group list so a draft or a
// changed Parent Group shows the values of the group it will land in. The
// chain uses the core-resolved `path` of each group, so a group treated as
// top level by the cycle rule ends the chain exactly as in the core.

import type { GroupView, Uuid } from '../../ipc';
import { t } from '../../i18n';

export interface InheritedEntry<T> {
  value: T;
  group: GroupView;
}

export interface InheritedSsh {
  port?: InheritedEntry<number>;
  username?: InheritedEntry<string>;
}

/** Maximum lookup depth (data-model 8.2). */
const MAX_LEVELS = 32;

/**
 * The groups that supply inherited values, nearest first: the start group,
 * then its ancestors. A missing group ends the chain.
 */
export function inheritanceChain(startGroupId: Uuid | null | undefined, groupsById: ReadonlyMap<Uuid, GroupView>): GroupView[] {
  if (startGroupId === null || startGroupId === undefined || startGroupId === '') {
    return [];
  }
  const start = groupsById.get(startGroupId);
  if (start === undefined) {
    return [];
  }
  const chain: GroupView[] = [start];
  for (const id of [...start.path].reverse()) {
    const group = groupsById.get(id);
    if (group === undefined || chain.length >= MAX_LEVELS) {
      break;
    }
    chain.push(group);
  }
  return chain;
}

/**
 * Port and username inherited by a record whose nearest group is
 * `startGroupId` (a host's group, or a group's parent).
 */
export function resolveInherited(startGroupId: Uuid | null | undefined, groupsById: ReadonlyMap<Uuid, GroupView>): InheritedSsh {
  const result: InheritedSsh = {};
  for (const group of inheritanceChain(startGroupId, groupsById)) {
    const ssh = group.data.ssh;
    if (result.port === undefined && typeof ssh?.port === 'number') {
      result.port = { value: ssh.port, group };
    }
    if (result.username === undefined && typeof ssh?.username === 'string' && ssh.username !== '') {
      result.username = { value: ssh.username, group };
    }
    if (result.port !== undefined && result.username !== undefined) {
      break;
    }
  }
  return result;
}

/** `"2222 (from group prod)"` (stage plan decision 16), or the fallback. */
export function inheritedPlaceholder<T extends string | number>(entry: InheritedEntry<T> | undefined, fallback = ''): string {
  if (entry === undefined) {
    return fallback;
  }
  return t('panels.inherited_placeholder', { value: entry.value, group: entry.group.data.name });
}
