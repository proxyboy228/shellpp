// Host context menu (UI 2.4) and group menu (UI 3.4) entries. The order of
// the spec is kept; items of later stages are left out (src/stage.ts):
// Quick Connect and Connect (`host-connect`, stage 2), Add to Workspace
// (`host-add-to-workspace`, stage 9), Collaborate (`host-collaborate`,
// stage 8), Copy to (`host-vault-transfer`, stage 8).

import { Copy, FolderInput, FolderOpen, Link, Pencil, Trash2 } from 'lucide-react';
import type { ContextMenuEntry, ContextMenuItem } from '../../app/ContextMenu/ContextMenu';
import type { GroupView, Uuid } from '../../ipc';
import { t } from '../../i18n';
import { sortedByPath } from './format';

/** Move to ▸ entries: Top level, then every group (minus `exclude`). */
export function moveToEntries(
  prefix: string,
  groups: readonly GroupView[],
  groupsById: ReadonlyMap<Uuid, GroupView>,
  onMove: (groupId: Uuid | null) => void,
  exclude: ReadonlySet<Uuid> = new Set(),
): ContextMenuEntry[] {
  const entries: ContextMenuEntry[] = [
    {
      id: 'top-level',
      label: t('hosts.menu.top_level'),
      onSelect: () => onMove(null),
      'data-testid': `${prefix}-top-level`,
    },
  ];
  const targets = sortedByPath(
    groups.filter((group) => !exclude.has(group.record_id)),
    groupsById,
  );
  if (targets.length > 0) {
    entries.push({ type: 'separator', id: 'groups-separator' });
  }
  for (const { group, label } of targets) {
    entries.push({
      id: `group-${group.record_id}`,
      label,
      onSelect: () => onMove(group.record_id),
      'data-testid': `${prefix}-group-${group.record_id}`,
    });
  }
  return entries;
}

export interface HostMenuActions {
  edit: (recordId: Uuid) => void;
  move: (recordIds: readonly Uuid[], groupId: Uuid | null) => void;
  duplicate: (recordId: Uuid) => void;
  copyLink: (recordId: Uuid) => void;
  remove: (recordIds: readonly Uuid[]) => void;
}

/**
 * The host context menu for the selected hosts. Edit, Duplicate and Copy
 * Link act on one host; Move to and Remove act on every selected host.
 */
export function hostMenuEntries(
  recordIds: readonly Uuid[],
  groups: readonly GroupView[],
  groupsById: ReadonlyMap<Uuid, GroupView>,
  actions: HostMenuActions,
): ContextMenuEntry[] {
  const single = recordIds.length === 1 ? recordIds[0] : undefined;
  const copyLink: ContextMenuItem = {
    id: 'copy-link',
    label: t('hosts.menu.copy_link'),
    icon: Link,
    info: t('hosts.menu.copy_link_info'),
    disabled: single === undefined,
    onSelect: () => {
      if (single !== undefined) {
        actions.copyLink(single);
      }
    },
    'data-testid': 'host-menu-copy-link',
  };
  return [
    {
      id: 'edit',
      label: t('hosts.menu.edit'),
      icon: Pencil,
      shortcut: 'E',
      disabled: single === undefined,
      onSelect: () => {
        if (single !== undefined) {
          actions.edit(single);
        }
      },
      'data-testid': 'host-menu-edit',
    },
    {
      id: 'move-to',
      label: t('hosts.menu.move_to'),
      icon: FolderInput,
      children: moveToEntries('host-menu-move-to', groups, groupsById, (groupId) => actions.move(recordIds, groupId)),
      'data-testid': 'host-menu-move-to',
    },
    {
      id: 'duplicate',
      label: t('hosts.menu.duplicate'),
      icon: Copy,
      disabled: single === undefined,
      onSelect: () => {
        if (single !== undefined) {
          actions.duplicate(single);
        }
      },
      'data-testid': 'host-menu-duplicate',
    },
    copyLink,
    { type: 'separator', id: 'remove-separator' },
    {
      id: 'remove',
      label: t('hosts.menu.remove'),
      icon: Trash2,
      danger: true,
      onSelect: () => actions.remove(recordIds),
      'data-testid': 'host-menu-remove',
    },
  ];
}

export interface GroupMenuActions {
  open: (groupId: Uuid) => void;
  edit: (groupId: Uuid) => void;
  move: (groupId: Uuid, parentId: Uuid | null) => void;
  remove: (groupId: Uuid) => void;
}

/**
 * The group menu (UI 3.4): Open, Edit, Move to, Remove. `prefix` is
 * `group-menu` on the card and `group-panel-menu` in the panel. Move to
 * excludes the group itself and its subtree (`exclude`).
 */
export function groupMenuEntries(
  prefix: string,
  group: GroupView,
  groups: readonly GroupView[],
  groupsById: ReadonlyMap<Uuid, GroupView>,
  exclude: ReadonlySet<Uuid>,
  actions: GroupMenuActions,
): ContextMenuEntry[] {
  const id = group.record_id;
  return [
    { id: 'open', label: t('hosts.group_menu.open'), icon: FolderOpen, onSelect: () => actions.open(id), 'data-testid': `${prefix}-open` },
    { id: 'edit', label: t('hosts.group_menu.edit'), icon: Pencil, onSelect: () => actions.edit(id), 'data-testid': `${prefix}-edit` },
    {
      id: 'move-to',
      label: t('hosts.group_menu.move_to'),
      icon: FolderInput,
      children: moveToEntries(`${prefix}-move-to`, groups, groupsById, (parentId) => actions.move(id, parentId), exclude),
      'data-testid': `${prefix}-move-to`,
    },
    { type: 'separator', id: 'remove-separator' },
    {
      id: 'remove',
      label: t('hosts.group_menu.remove'),
      icon: Trash2,
      danger: true,
      onSelect: () => actions.remove(id),
      'data-testid': `${prefix}-remove`,
    },
  ];
}
