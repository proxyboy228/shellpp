// Unlock with the recovery key (UI 13.2, local mode): the recovery key, a new
// master password and its confirmation. On success the app unlocks with the
// new password.
//
// Owner: W4.

import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage, validationMessage } from '../../app/errors';
import { requestResync } from '../../app/session';
import { updateView } from '../../app/store';
import { Button, PasswordInput, TextInput } from '../../components';
import { t } from '../../i18n';
import { recoveryUnlock, toIpcError } from '../../ipc';
import styles from './LockScreens.module.css';

export function RecoveryUnlockScreen() {
  const [recoveryKey, setRecoveryKey] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const keyRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    keyRef.current?.focus();
  }, []);

  const passwordsFilled = password !== '' && confirm !== '';
  const mismatch = passwordsFilled && password !== confirm;
  const canSubmit = recoveryKey.trim() !== '' && passwordsFilled && !mismatch && !busy;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canSubmit) {
      return;
    }
    setBusy(true);
    setKeyError(null);
    setPasswordError(null);
    setFormError(null);
    try {
      await recoveryUnlock({ recovery_key: recoveryKey, new_password: password });
      setRecoveryKey('');
      setPassword('');
      setConfirm('');
      updateView({ unlockMode: 'password' });
      requestResync();
    } catch (reason) {
      const error = toIpcError(reason);
      if (error.code === 'invalid_recovery_key') {
        setKeyError(t('recovery.wrong_key'));
        keyRef.current?.focus();
      } else if (error.code === 'validation_failed') {
        const keyReason = error.fieldReason('recovery_key');
        const passwordReason = error.fieldReason('new_password');
        if (keyReason !== undefined) {
          setKeyError(keyReason === 'invalid' ? t('recovery.invalid_format') : validationMessage(keyReason));
        }
        if (passwordReason !== undefined) {
          setPasswordError(passwordReason === 'too_short' ? t('recovery.too_short') : validationMessage(passwordReason));
        }
      } else {
        setFormError(errorMessage(error));
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.screen}>
      <form className={styles.card} onSubmit={(event) => void submit(event)} data-testid="recovery-unlock-screen" noValidate>
        <p className={styles.appName}>{t('app.name')}</p>
        <h1 className={styles.title}>{t('recovery.title')}</h1>
        <p className={styles.subtitle}>{t('recovery.subtitle')}</p>
        <div className={styles.fields}>
          <TextInput
            ref={keyRef}
            label={t('recovery.key')}
            mono
            value={recoveryKey}
            placeholder={t('recovery.key_placeholder')}
            autoComplete="off"
            spellCheck={false}
            autoCapitalize="characters"
            onChange={(event) => {
              setRecoveryKey(event.target.value);
              setKeyError(null);
            }}
            error={keyError ?? undefined}
            data-testid="recovery-unlock-key"
          />
          <PasswordInput
            label={t('recovery.new_password')}
            value={password}
            onChange={(event) => {
              setPassword(event.target.value);
              setPasswordError(null);
            }}
            error={passwordError ?? undefined}
            toggleLabel={t('common.show_password')}
            data-testid="recovery-unlock-password"
          />
          <PasswordInput
            label={t('recovery.confirm')}
            value={confirm}
            onChange={(event) => setConfirm(event.target.value)}
            error={mismatch ? t('recovery.mismatch') : undefined}
            toggleLabel={t('common.show_password')}
            data-testid="recovery-unlock-confirm"
          />
        </div>
        {formError === null ? null : (
          <p className={styles.formError} role="alert">
            {formError}
          </p>
        )}
        <Button
          type="submit"
          variant="primary"
          fullWidth
          disabled={!canSubmit && !busy}
          loading={busy}
          data-testid="recovery-unlock-submit"
        >
          {t('recovery.submit')}
        </Button>
        <Button
          variant="text"
          className={styles.link}
          onClick={() => updateView({ unlockMode: 'password' })}
          data-testid="recovery-unlock-back"
        >
          {t('recovery.back')}
        </Button>
      </form>
    </div>
  );
}

export default RecoveryUnlockScreen;
