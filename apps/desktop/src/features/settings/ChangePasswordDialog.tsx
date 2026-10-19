// Change master password (UI 13.4, local mode): the current password, the new
// password and its confirmation. The change takes effect at once. The warning
// about other devices is a sync feature (stage 7) and is not rendered.
//
// Owner: W4.

import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage, validationMessage } from '../../app/errors';
import { showToast } from '../../app/toast';
import { Button, Modal, PasswordInput } from '../../components';
import { t } from '../../i18n';
import { passwordChange, toIpcError } from '../../ipc';
import { isVisible } from '../../stage';
import { stopKeyPropagation } from './nestedModal';
import styles from './Settings.module.css';

export interface ChangePasswordDialogProps {
  open: boolean;
  onClose: () => void;
}

function ChangePasswordForm({ onClose }: { onClose: () => void }) {
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [currentError, setCurrentError] = useState<string | null>(null);
  const [nextError, setNextError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const currentRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    currentRef.current?.focus();
  }, []);

  const mismatch = next !== '' && confirm !== '' && next !== confirm;
  const canSubmit = current !== '' && next !== '' && confirm !== '' && !mismatch && !busy;

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (!canSubmit) {
      return;
    }
    setBusy(true);
    setCurrentError(null);
    setNextError(null);
    setFormError(null);
    try {
      await passwordChange({ current_password: current, new_password: next });
      setCurrent('');
      setNext('');
      setConfirm('');
      onClose();
      showToast({ message: t('security.password_changed'), level: 'success', testId: 'security-password-changed-toast' });
    } catch (reason) {
      const error = toIpcError(reason);
      if (error.code === 'invalid_password') {
        setCurrent('');
        setCurrentError(t('change_password.wrong_current'));
        currentRef.current?.focus();
      } else if (error.code === 'validation_failed') {
        const nextReason = error.fieldReason('new_password');
        setNextError(
          nextReason === 'too_short'
            ? t('change_password.too_short')
            : validationMessage(nextReason ?? error.fieldErrors[0]?.reason ?? 'invalid'),
        );
      } else {
        setFormError(errorMessage(error));
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className={styles.dialogForm} onSubmit={(event) => void submit(event)} noValidate>
      {isVisible('security-password-change-sync-warning') ? (
        <p className={styles.notice} data-testid="change-password-sync-warning">
          {t('change_password.sync_warning')}
        </p>
      ) : null}
      <PasswordInput
        ref={currentRef}
        label={t('change_password.current')}
        value={current}
        onChange={(event) => {
          setCurrent(event.target.value);
          setCurrentError(null);
        }}
        error={currentError ?? undefined}
        toggleLabel={t('common.show_password')}
        data-testid="change-password-current"
      />
      <PasswordInput
        label={t('change_password.new')}
        value={next}
        onChange={(event) => {
          setNext(event.target.value);
          setNextError(null);
        }}
        error={nextError ?? undefined}
        toggleLabel={t('common.show_password')}
        data-testid="change-password-new"
      />
      <PasswordInput
        label={t('change_password.confirm')}
        value={confirm}
        onChange={(event) => setConfirm(event.target.value)}
        error={mismatch ? t('change_password.mismatch') : undefined}
        toggleLabel={t('common.show_password')}
        data-testid="change-password-confirm"
      />
      {formError === null ? null : (
        <p className={styles.formError} role="alert">
          {formError}
        </p>
      )}
      <div className={styles.dialogActions}>
        <Button onClick={onClose} data-testid="change-password-cancel">
          {t('common.cancel')}
        </Button>
        <Button
          type="submit"
          variant="primary"
          disabled={!canSubmit && !busy}
          loading={busy}
          data-testid="change-password-submit"
        >
          {t('change_password.submit')}
        </Button>
      </div>
    </form>
  );
}

export function ChangePasswordDialog({ open, onClose }: ChangePasswordDialogProps) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('change_password.title')}
      closeLabel={t('common.close')}
      size="small"
      onKeyDown={stopKeyPropagation}
      data-testid="change-password-dialog"
    >
      <ChangePasswordForm onClose={onClose} />
    </Modal>
  );
}

export default ChangePasswordDialog;
