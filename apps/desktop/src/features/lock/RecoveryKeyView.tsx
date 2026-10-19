// The recovery key display shared by first run (UI 13.1) and the Security
// settings (UI 13.4): the key in a monospace font with a Copy button and,
// where required, the "I saved the recovery key" checkbox.
//
// Copy goes through `secretCopy({kind: 'recovery_key'})`: Rust copies the key
// and clears the clipboard after 30 seconds. A native copy of the selected key
// text takes the same path (the element is `data-sensitive`).
//
// Owner: W4.

import { Copy } from 'lucide-react';
import { useState } from 'react';
import { showErrorToast } from '../../app/errors';
import { showToast } from '../../app/toast';
import { Button, Checkbox } from '../../components';
import { t } from '../../i18n';
import { isIpcError, secretCopy } from '../../ipc';
import styles from './LockScreens.module.css';

export interface RecoveryKeyViewProps {
  recoveryKey: string;
  /** Test ids: the key text, the Copy button, the toast after copying and the checkbox. */
  testIds: { key: string; copy: string; copiedToast: string; saved?: string };
  /** Shows the required checkbox when `saved` and `onSavedChange` are given. */
  saved?: boolean;
  onSavedChange?: (saved: boolean) => void;
}

export function RecoveryKeyView({ recoveryKey, testIds, saved, onSavedChange }: RecoveryKeyViewProps) {
  const [copying, setCopying] = useState(false);

  const copy = async () => {
    setCopying(true);
    try {
      await secretCopy({ kind: 'recovery_key' });
      showToast({ message: t('recovery_key.copied'), level: 'success', testId: testIds.copiedToast });
    } catch (reason) {
      if (isIpcError(reason, 'wrong_state')) {
        showToast({ message: t('recovery_key.copy_expired'), level: 'warning', testId: 'app-error-toast' });
      } else {
        showErrorToast(reason);
      }
    } finally {
      setCopying(false);
    }
  };

  return (
    <div className={styles.recoveryKey}>
      <div className={styles.keyBox}>
        {/* Selecting and copying the key (Ctrl+C) uses the protected copy too. */}
        <code
          className={styles.keyText}
          data-sensitive=""
          onCopy={(event) => {
            event.preventDefault();
            void copy();
          }}
          onCut={(event) => event.preventDefault()}
          data-testid={testIds.key}
        >
          {recoveryKey}
        </code>
        <Button
          icon={<Copy />}
          className={styles.copyButton}
          loading={copying}
          onClick={() => void copy()}
          data-testid={testIds.copy}
        >
          {t('common.copy')}
        </Button>
      </div>
      {saved !== undefined && onSavedChange !== undefined ? (
        <Checkbox
          checked={saved}
          onCheckedChange={onSavedChange}
          label={t('recovery_key.saved')}
          data-testid={testIds.saved}
        />
      ) : null}
    </div>
  );
}
