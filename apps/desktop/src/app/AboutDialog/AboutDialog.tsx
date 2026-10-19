// About dialog (UI 1.4, Help > About): name, version, license and the
// repository link (opened in the system browser).
//
// Owner: W4.

import { ExternalLink } from 'lucide-react';
import { Button, Modal } from '../../components';
import { t } from '../../i18n';
import { externalOpenUrl } from '../../ipc';
import { showErrorToast } from '../errors';
import { REPOSITORY_URL } from '../links';
import { useAppInfo } from '../store';
import styles from './AboutDialog.module.css';

export interface AboutDialogProps {
  open: boolean;
  onClose: () => void;
}

export function AboutDialog({ open, onClose }: AboutDialogProps) {
  const info = useAppInfo();
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('about.title')}
      closeLabel={t('common.close')}
      size="small"
      data-testid="about-dialog"
      footer={
        <Button variant="primary" onClick={onClose}>
          {t('common.close')}
        </Button>
      }
    >
      <div className={styles.content}>
        <p className={styles.name}>{t('app.name')}</p>
        <p className={styles.version} data-testid="about-dialog-version">
          {t('about.version', { version: info?.version ?? '' })}
        </p>
        <p className={styles.text}>{t('about.description')}</p>
        <p className={styles.text}>{t('about.license')}</p>
        <Button
          variant="text"
          icon={<ExternalLink />}
          onClick={() => externalOpenUrl(REPOSITORY_URL).catch(showErrorToast)}
          data-testid="about-dialog-repository"
        >
          {t('about.repository')}
        </Button>
      </div>
    </Modal>
  );
}

export default AboutDialog;
