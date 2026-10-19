// The permanent warning of first run (UI 13.1): without the master password
// and the recovery key nobody can recover the data. It cannot be dismissed.
//
// Owner: W4.

import { TriangleAlert } from 'lucide-react';
import { t } from '../../i18n';
import styles from './LockScreens.module.css';

export function LockWarning({ 'data-testid': testId }: { 'data-testid'?: string }) {
  return (
    <div className={styles.warning} role="note" data-testid={testId}>
      <TriangleAlert className={styles.warningIcon} size={16} strokeWidth={1.75} aria-hidden="true" />
      <p className={styles.warningText}>{t('lock.warning')}</p>
    </div>
  );
}
