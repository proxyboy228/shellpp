// Record actions of the Hosts screen (UI 2.4, 3.3, 3.4): remove with undo,
// duplicate, copy link, move, create a group. Failures show an error toast.

import {
  clipboardWriteText,
  recordDelete,
  recordDuplicate,
  recordMove,
  recordRestore,
  recordSave,
  toIpcError,
} from '../../ipc';
import type { GroupView, HostView, MovePosition, Uuid } from '../../ipc';
import { t } from '../../i18n';
import { showToast } from '../../app/toast';

/** The undo toast stays for 8 seconds (stage plan decision 14). */
export const UNDO_TOAST_MS = 8000;

/** `shellpp://host/<record_id>` (architecture 7). */
export function hostLink(recordId: Uuid): string {
  return `shellpp://host/${recordId}`;
}

export function showErrorToast(reason: unknown): void {
  showToast({ message: t(`errors.${toIpcError(reason).code}`), level: 'error', testId: 'hosts-error-toast' });
}

/** Removes hosts, then offers Undo for 8 seconds. Resolves with the removed ids. */
export async function removeHosts(vaultId: Uuid, recordIds: readonly Uuid[]): Promise<Uuid[]> {
  try {
    const result = await recordDelete({ vault_id: vaultId, record_ids: [...recordIds] });
    const deleted = result.deleted;
    let undone = false;
    const dismiss = showToast({
      message: deleted.length === 1 ? t('hosts.toast.removed_one') : t('hosts.toast.removed_other', { count: deleted.length }),
      level: 'info',
      durationMs: UNDO_TOAST_MS,
      testId: 'hosts-undo-toast',
      action: {
        label: t('hosts.toast.undo'),
        testId: 'hosts-undo-toast-action',
        onAction: () => {
          if (undone) {
            return;
          }
          undone = true;
          dismiss();
          recordRestore({ vault_id: vaultId, record_ids: deleted }).catch(showErrorToast);
        },
      },
    });
    return deleted;
  } catch (reason) {
    showErrorToast(reason);
    return [];
  }
}

/** Removes a group; its children move one level up (done by the core). */
export async function removeGroup(vaultId: Uuid, groupId: Uuid): Promise<boolean> {
  try {
    await recordDelete({ vault_id: vaultId, record_ids: [groupId] });
    return true;
  } catch (reason) {
    showErrorToast(reason);
    return false;
  }
}

/** Duplicates a host (`label + " Copy"`, right after the original). */
export async function duplicateHost(vaultId: Uuid, recordId: Uuid): Promise<HostView | null> {
  try {
    const view = await recordDuplicate({ vault_id: vaultId, record_id: recordId });
    return view.type === 'host' ? view : null;
  } catch (reason) {
    showErrorToast(reason);
    return null;
  }
}

/** Copies the host link to the clipboard (UI 2.4 Copy Link). */
export async function copyHostLink(recordId: Uuid): Promise<void> {
  try {
    await clipboardWriteText(hostLink(recordId));
    showToast({ message: t('hosts.toast.link_copied'), level: 'success', testId: 'hosts-link-copied-toast' });
  } catch (reason) {
    showErrorToast(reason);
  }
}

export async function moveRecords(
  vaultId: Uuid,
  recordIds: readonly Uuid[],
  targetGroupId: Uuid | null,
  position: MovePosition = { kind: 'end' },
): Promise<boolean> {
  try {
    await recordMove({ vault_id: vaultId, record_ids: [...recordIds], target_group_id: targetGroupId, position });
    return true;
  } catch (reason) {
    showErrorToast(reason);
    return false;
  }
}

/** Creates a group (New Group with a name, or "create on the fly" in the host panel). */
export async function createGroup(vaultId: Uuid, name: string, parentId: Uuid | null): Promise<GroupView | null> {
  try {
    const view = await recordSave({
      vault_id: vaultId,
      record_id: null,
      type: 'group',
      data: parentId === null ? { name } : { name, parent_id: parentId },
    });
    return view.type === 'group' ? view : null;
  } catch (reason) {
    showErrorToast(reason);
    return null;
  }
}
