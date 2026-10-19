// Master password prompt before showing or reissuing the recovery key
// (UI 13.4; docs/security.md 8: both need the password again even while
// unlocked).
//
// Owner: W4.

import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage } from '../../app/errors';
import { Button, Modal, PasswordInput } from '../../components';
import { t } from '../../i18n';
import { recoveryKeyReissue, recoveryKeyShow, toIpcError } from '../../ipc';
import { stopKeyPropagation } from './nestedModal';
import styles from './Settings.module.css';

export type RecoveryMode = 'show' | 'reissue';

export interface RecoveryPasswordDialogProps {
  mode: RecoveryMode;
  onCancel: () => void;
  /** Called with the recovery key text (the new key for `reissue`). */
  onSuccess: (recoveryKey: string) => void;
}

export function RecoveryPasswordDialog({ mode, onCancel, onSuccess }: RecoveryPasswordDialogProps) {
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (password === '' || busy) {
      return;
    }
    const candidate = password;
    setPassword('');
    setBusy(true);
    setError(null);
    try {
      const result = mode === 'show' ? await recoveryKeyShow(candidate) : await recoveryKeyReissue(candidate);
      onSuccess(result.recovery_key);
    } catch (reason) {
      const failure = toIpcError(reason);
      setError(failure.code === 'invalid_password' ? t('recovery_password.wrong') : errorMessage(failure));
      inputRef.current?.focus();
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={onCancel}
      title={mode === 'show' ? t('recovery_password.title_show') : t('recovery_password.title_reissue')}
      closeLabel={t('common.close')}
      size="small"
      onKeyDown={stopKeyPropagation}
      data-testid="recovery-password-dialog"
    >
      <form className={styles.dialogForm} onSubmit={(event) => void submit(event)} noValidate>
        <p className={styles.dialogText}>
          {mode === 'show' ? t('recovery_password.text_show') : t('recovery_password.text_reissue')}
        </p>
        <PasswordInput
          ref={inputRef}
          label={t('recovery_password.input')}
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            setError(null);
          }}
          error={error ?? undefined}
          toggleLabel={t('common.show_password')}
          data-testid="recovery-password-input"
        />
        <div className={styles.dialogActions}>
          <Button onClick={onCancel} data-testid="recovery-password-cancel">
            {t('common.cancel')}
          </Button>
          <Button
            type="submit"
            variant={mode === 'reissue' ? 'danger' : 'primary'}
            disabled={password === '' && !busy}
            loading={busy}
            data-testid="recovery-password-submit"
          >
            {mode === 'show' ? t('recovery_password.submit_show') : t('recovery_password.submit_reissue')}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

export default RecoveryPasswordDialog;
