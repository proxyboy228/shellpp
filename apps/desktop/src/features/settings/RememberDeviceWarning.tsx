// Warning before turning on "Remember on this device" (UI 13.4): any program
// running under this OS user will be able to unlock the data.
//
// Owner: W4.

import { TriangleAlert } from 'lucide-react';
import { Button, Modal } from '../../components';
import { t } from '../../i18n';
import { stopKeyPropagation } from './nestedModal';
import styles from './Settings.module.css';

export interface RememberDeviceWarningProps {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RememberDeviceWarning({ open, onConfirm, onCancel }: RememberDeviceWarningProps) {
  return (
    <Modal
      open={open}
      onClose={onCancel}
      title={t('remember_device.title')}
      closeLabel={t('common.close')}
      size="small"
      onKeyDown={stopKeyPropagation}
      data-testid="remember-device-warning-dialog"
      footer={
        <>
          <Button onClick={onCancel} data-testid="remember-device-warning-cancel">
            {t('common.cancel')}
          </Button>
          <Button variant="primary" onClick={onConfirm} data-testid="remember-device-warning-confirm">
            {t('remember_device.confirm')}
          </Button>
        </>
      }
    >
      <div className={styles.warning} role="note">
        <TriangleAlert className={styles.warningIcon} size={16} strokeWidth={1.75} aria-hidden="true" />
        <p className={styles.dialogText}>{t('remember_device.text')}</p>
      </div>
    </Modal>
  );
}

export default RememberDeviceWarning;
