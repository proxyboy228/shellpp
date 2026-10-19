// First run (UI 13.1): create the master password, then save the recovery
// key. The window stays on the recovery key step until Finish even though the
// core is already unlocked (plan, "Routing").
//
// Owner: W4.

import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage, validationMessage } from '../../app/errors';
import { requestResync } from '../../app/session';
import { updateView } from '../../app/store';
import { Button, PasswordInput } from '../../components';
import { t } from '../../i18n';
import { firstRunSetup, toIpcError } from '../../ipc';
import { LockWarning } from './LockWarning';
import { RecoveryKeyView } from './RecoveryKeyView';
import styles from './LockScreens.module.css';

function PasswordStep({ onDone }: { onDone: (recoveryKey: string) => void }) {
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const passwordRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    passwordRef.current?.focus();
  }, []);

  const bothFilled = password !== '' && confirm !== '';
  const mismatch = bothFilled && password !== confirm;
  const canContinue = bothFilled && !mismatch && !busy;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canContinue) {
      return;
    }
    setBusy(true);
    setPasswordError(null);
    setFormError(null);
    // Keep this window on the first-run screen when `lock:changed` arrives.
    updateView({ firstRunInProgress: true });
    try {
      const { recovery_key } = await firstRunSetup(password);
      setPassword('');
      setConfirm('');
      onDone(recovery_key);
    } catch (reason) {
      updateView({ firstRunInProgress: false });
      const error = toIpcError(reason);
      const fieldReason = error.fieldReason('password');
      if (fieldReason === 'too_short') {
        setPasswordError(t('first_run.too_short'));
      } else if (fieldReason !== undefined) {
        setPasswordError(validationMessage(fieldReason));
      } else {
        setFormError(errorMessage(error));
      }
      passwordRef.current?.focus();
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className={styles.card} onSubmit={(event) => void submit(event)} data-testid="first-run-screen" noValidate>
      <p className={styles.appName}>{t('app.name')}</p>
      <h1 className={styles.title}>{t('first_run.title')}</h1>
      <p className={styles.subtitle}>{t('first_run.subtitle')}</p>
      <div className={styles.fields}>
        <PasswordInput
          ref={passwordRef}
          label={t('first_run.password')}
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            setPasswordError(null);
          }}
          error={passwordError ?? undefined}
          toggleLabel={t('common.show_password')}
          data-testid="first-run-password"
        />
        <PasswordInput
          label={t('first_run.confirm')}
          value={confirm}
          onChange={(event) => setConfirm(event.target.value)}
          error={mismatch ? t('first_run.mismatch') : undefined}
          toggleLabel={t('common.show_password')}
          data-testid="first-run-confirm"
        />
      </div>
      <LockWarning data-testid="first-run-warning" />
      {formError === null ? null : (
        <p className={styles.formError} role="alert">
          {formError}
        </p>
      )}
      <Button type="submit" variant="primary" fullWidth disabled={!canContinue && !busy} loading={busy} data-testid="first-run-continue">
        {t('first_run.continue')}
      </Button>
    </form>
  );
}

function RecoveryStep({ recoveryKey }: { recoveryKey: string }) {
  const [saved, setSaved] = useState(false);

  const finish = () => {
    if (!saved) {
      return;
    }
    updateView({ firstRunInProgress: false });
    requestResync();
  };

  return (
    <div className={styles.card} data-testid="first-run-recovery-step">
      <p className={styles.appName}>{t('app.name')}</p>
      <h1 className={styles.title}>{t('first_run.recovery_title')}</h1>
      <p className={styles.subtitle}>{t('first_run.recovery_text')}</p>
      <RecoveryKeyView
        recoveryKey={recoveryKey}
        saved={saved}
        onSavedChange={setSaved}
        testIds={{
          key: 'first-run-recovery-key',
          copy: 'first-run-recovery-copy',
          copiedToast: 'first-run-copied-toast',
          saved: 'first-run-recovery-saved',
        }}
      />
      <LockWarning data-testid="first-run-warning" />
      <Button variant="primary" fullWidth disabled={!saved} onClick={finish} data-testid="first-run-finish">
        {t('first_run.finish')}
      </Button>
    </div>
  );
}

export function FirstRunScreen() {
  const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
  return (
    <div className={styles.screen}>
      {recoveryKey === null ? <PasswordStep onDone={setRecoveryKey} /> : <RecoveryStep recoveryKey={recoveryKey} />}
    </div>
  );
}

export default FirstRunScreen;
