// Removal confirmations (UI 2.4 for hosts, 3.4 for groups).

import { Button, Modal } from '../../components';
import type { GroupView } from '../../ipc';
import { t } from '../../i18n';
import styles from './HostsScreen.module.css';

export interface HostRemoveDialogProps {
  /** Titles of the hosts to remove. */
  titles: readonly string[];
  onConfirm: () => void;
  onCancel: () => void;
}

export function HostRemoveDialog({ titles, onConfirm, onCancel }: HostRemoveDialogProps) {
  const count = titles.length;
  return (
    <Modal
      open
      size="small"
      title={count === 1 ? t('hosts.remove_dialog.title_one') : t('hosts.remove_dialog.title_many', { count })}
      closeLabel={t('hosts.dialog.close')}
      onClose={onCancel}
      data-count={count}
      data-testid="host-remove-dialog"
      footer={
        <>
          <Button onClick={onCancel} data-testid="host-remove-dialog-cancel">
            {t('hosts.dialog.cancel')}
          </Button>
          <Button variant="danger" onClick={onConfirm} data-testid="host-remove-dialog-confirm">
            {t('hosts.dialog.remove')}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText}>
        {count === 1
          ? t('hosts.remove_dialog.text_one', { title: titles[0] ?? '' })
          : t('hosts.remove_dialog.text_many', { count })}
      </p>
      <p className={styles.dialogHint}>{t('hosts.remove_dialog.undo_hint')}</p>
    </Modal>
  );
}

export interface GroupRemoveDialogProps {
  group: GroupView;
  onConfirm: () => void;
  onCancel: () => void;
}

export function GroupRemoveDialog({ group, onConfirm, onCancel }: GroupRemoveDialogProps) {
  return (
    <Modal
      open
      size="small"
      title={t('hosts.group_remove_dialog.title')}
      closeLabel={t('hosts.dialog.close')}
      onClose={onCancel}
      data-testid="group-remove-dialog"
      footer={
        <>
          <Button onClick={onCancel} data-testid="group-remove-dialog-cancel">
            {t('hosts.dialog.cancel')}
          </Button>
          <Button variant="danger" onClick={onConfirm} data-testid="group-remove-dialog-confirm">
            {t('hosts.dialog.remove')}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText}>{t('hosts.group_remove_dialog.text', { name: group.data.name })}</p>
      {group.child_count > 0 ? (
        <p className={styles.dialogHint} data-testid="group-remove-dialog-non-empty">
          {t('hosts.group_remove_dialog.non_empty')}
        </p>
      ) : null}
    </Modal>
  );
}
