// Recovery key display in the Security settings (UI 13.4). After Reissue the
// new key is shown as at first run, with the required "I saved it" checkbox:
// Done (and closing) waits for it. After Show, Done is always enabled.
//
// Owner: W4.

import { useState } from 'react';
import { Button, Modal } from '../../components';
import { t } from '../../i18n';
import { RecoveryKeyView } from '../lock/RecoveryKeyView';
import type { RecoveryMode } from './RecoveryPasswordDialog';
import { stopKeyPropagation } from './nestedModal';
import styles from './Settings.module.css';

export interface RecoveryKeyDialogProps {
  mode: RecoveryMode;
  recoveryKey: string;
  onDone: () => void;
}

export function RecoveryKeyDialog({ mode, recoveryKey, onDone }: RecoveryKeyDialogProps) {
  const [saved, setSaved] = useState(false);
  const required = mode === 'reissue';
  const canClose = !required || saved;
  const close = () => {
    if (canClose) {
      onDone();
    }
  };
  return (
    <Modal
      open
      onClose={close}
      title={required ? t('recovery_key_dialog.title_new') : t('recovery_key_dialog.title')}
      closeLabel={t('common.close')}
      size="medium"
      onKeyDown={stopKeyPropagation}
      data-testid="recovery-key-dialog"
      footer={
        <Button variant="primary" disabled={!canClose} onClick={close} data-testid="recovery-key-done">
          {t('common.done')}
        </Button>
      }
    >
      <div className={styles.dialogForm}>
        <p className={styles.dialogText}>
          {required ? t('recovery_key_dialog.text_new') : t('recovery_key_dialog.text')}
        </p>
        <RecoveryKeyView
          recoveryKey={recoveryKey}
          saved={required ? saved : undefined}
          onSavedChange={required ? setSaved : undefined}
          testIds={{
            key: 'recovery-key-text',
            copy: 'recovery-key-copy',
            copiedToast: 'recovery-key-copied-toast',
            saved: 'recovery-key-saved',
          }}
        />
      </div>
    </Modal>
  );
}

export default RecoveryKeyDialog;
