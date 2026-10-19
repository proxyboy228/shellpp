// Group panel (UI 3.4) in the details panel: Name (required), Parent Group,
// Color and the inheritable SSH block (username, port) with "from group X"
// placeholders. Autosave as for hosts.

import { useEffect, useMemo, useRef, useState } from 'react';
import { DetailsPanel } from '../../app/DetailsPanel/DetailsPanel';
import { TextInput } from '../../components';
import { recordSave } from '../../ipc';
import type { GroupData, GroupView, Uuid } from '../../ipc';
import { t } from '../../i18n';
import { subtreeIds } from '../hosts/format';
import { groupMenuEntries, type GroupMenuActions } from '../hosts/menus';
import { reasonFor } from './form';
import { GROUP_FIELDS, type GroupForm } from './fields';
import { inheritedPlaceholder, resolveInherited } from './inheritance';
import { usePanelRootAttributes } from './panelAttributes';
import { useRecordForm } from './useRecordForm';
import { ColorField } from './fields/ColorField';
import { GroupPicker } from './fields/GroupPicker';
import styles from './panels.module.css';

export const GROUP_PANEL_TEST_ID = 'group-panel';
const TID = GROUP_PANEL_TEST_ID;

export interface GroupPanelProps {
  vaultId: Uuid;
  /** `null`: a new group (a draft until the name is valid). */
  recordId: Uuid | null;
  view: GroupView | undefined;
  /** Parent of a new group (the open folder). */
  draftParentId: Uuid | null;
  groups: readonly GroupView[];
  groupsById: ReadonlyMap<Uuid, GroupView>;
  /** Each new value moves focus to Name. */
  focusNonce: number;
  onCreated: (view: GroupView) => void;
  onClose: () => void;
  /** Open, Move to and Remove of the "..." menu; Edit focuses Name. */
  actions: Omit<GroupMenuActions, 'edit'>;
  onMissing: () => void;
}

export function GroupPanel({
  vaultId,
  recordId: initialRecordId,
  view,
  draftParentId,
  groups,
  groupsById,
  focusNonce,
  onCreated,
  onClose,
  actions,
  onMissing,
}: GroupPanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  const record = useRecordForm<GroupForm, GroupView>({
    table: GROUP_FIELDS,
    requiredKey: 'name',
    recordId: initialRecordId,
    view,
    draftData: draftParentId === null ? {} : { parent_id: draftParentId },
    persist: async (recordId, data) => {
      const saved = await recordSave({ vault_id: vaultId, record_id: recordId, type: 'group', data: data as GroupData });
      if (saved.type !== 'group') {
        throw new Error('record_save returned another type');
      }
      return saved;
    },
    onSaved: (saved, created) => {
      if (created) {
        onCreated(saved);
      }
    },
    onMissing,
  });
  const { form, errors, update, recordId, base } = record;

  usePanelRootAttributes(containerRef, TID, {
    'data-record-id': recordId ?? undefined,
    'data-save-state': record.saveState,
  });

  // Focus requests: from the screen (creation, Edit in the card menu) and from
  // Edit in this panel's menu.
  const [localFocus, setLocalFocus] = useState(0);
  useEffect(() => {
    if (focusNonce > 0 || localFocus > 0) {
      nameRef.current?.focus();
    }
  }, [focusNonce, localFocus]);

  const message = (path: string): string | undefined => {
    const reason = reasonFor(errors, path);
    return reason === undefined ? undefined : t(`validation.${reason}`);
  };

  // The group itself and its subtree cannot become its parent.
  const exclude = useMemo(() => (recordId === null ? new Set<Uuid>() : subtreeIds(recordId, groups)), [recordId, groups]);
  const inherited = useMemo(() => resolveInherited(form.parent_id, groupsById), [form.parent_id, groupsById]);

  const menuItems =
    base === null
      ? []
      : groupMenuEntries(`${TID}-menu`, base, groups, groupsById, exclude, {
          ...actions,
          edit: () => setLocalFocus((count) => count + 1),
        });

  return (
    <div ref={containerRef} className={styles.panelSlot}>
      <DetailsPanel
        title={recordId === null ? t('group_panel.title_new') : t('group_panel.title')}
        menuItems={menuItems}
        onCollapse={onClose}
        data-testid={TID}
        data-record-id={recordId ?? undefined}
        data-save-state={record.saveState}
      >
        <div className={styles.form}>
          {/* `panel-vault-field` (stage 8) is hidden. */}
          <TextInput
            ref={nameRef}
            label={t('group_panel.name')}
            value={form.name}
            autoComplete="off"
            error={message('name')}
            onChange={(event) => update('name', event.target.value)}
            data-testid={`${TID}-name`}
          />
          <GroupPicker
            label={t('group_panel.parent')}
            value={form.parent_id}
            onChange={(parentId) => update('parent_id', parentId)}
            groups={groups}
            groupsById={groupsById}
            exclude={exclude}
            data-testid={`${TID}-parent`}
          />
          <ColorField value={form.color} onChange={(color) => update('color', color)} data-testid={`${TID}-color`} />
          <section className={styles.block} aria-labelledby={`${TID}-ssh-title`}>
            <div className={styles.sshHeader}>
              <h3 id={`${TID}-ssh-title`} className={styles.blockTitle}>
                {t('group_panel.ssh')}
              </h3>
              <TextInput
                className={styles.portField}
                aria-label={t('group_panel.port')}
                inputMode="numeric"
                autoComplete="off"
                placeholder={inheritedPlaceholder(inherited.port, '22')}
                value={form.port}
                error={message('ssh.port')}
                onChange={(event) => update('port', event.target.value)}
                data-testid={`${TID}-ssh-port`}
              />
            </div>
            <TextInput
              label={t('group_panel.username')}
              autoComplete="off"
              spellCheck={false}
              placeholder={inheritedPlaceholder(inherited.username)}
              value={form.username}
              error={message('ssh.username')}
              onChange={(event) => update('username', event.target.value)}
              data-testid={`${TID}-ssh-username`}
            />
            <p className={styles.hint}>{t('group_panel.inheritance_hint')}</p>
            {/* `group-credentials` (stage 3) and `host-chaining` (stage 5) are hidden. */}
          </section>
        </div>
      </DetailsPanel>
    </div>
  );
}

export default GroupPanel;
