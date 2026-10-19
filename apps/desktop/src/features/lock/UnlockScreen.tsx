// Unlock screen (UI 13.2): shown at launch and after a lock. It covers the
// whole window content; only the window buttons stay. The field is cleared as
// soon as the password is sent; a wrong password shows an error under the
// field and keeps focus in it.
//
// Owner: W4.

import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage } from '../../app/errors';
import { requestResync } from '../../app/session';
import { updateView } from '../../app/store';
import { Button, PasswordInput } from '../../components';
import { t } from '../../i18n';
import { toIpcError, unlock } from '../../ipc';
import styles from './LockScreens.module.css';

export function UnlockScreen() {
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
    setError(null);
    setBusy(true);
    try {
      await unlock(candidate);
      requestResync();
    } catch (reason) {
      const failure = toIpcError(reason);
      setError(failure.code === 'invalid_password' ? t('unlock.wrong_password') : errorMessage(failure));
    } finally {
      setBusy(false);
      inputRef.current?.focus();
    }
  };

  return (
    <div className={styles.screen}>
      <form className={styles.card} onSubmit={(event) => void submit(event)} data-testid="unlock-screen" noValidate>
        <h1 className={styles.appTitle} data-testid="unlock-app-name">
          {t('app.name')}
        </h1>
        <div className={styles.fields}>
          <PasswordInput
            ref={inputRef}
            label={t('unlock.password')}
            value={password}
            onChange={(event) => {
              setPassword(event.target.value);
              setError(null);
            }}
            error={error ?? undefined}
            toggleLabel={t('common.show_password')}
            data-testid="unlock-password"
          />
        </div>
        <Button
          type="submit"
          variant="primary"
          fullWidth
          disabled={password === '' && !busy}
          loading={busy}
          data-testid="unlock-submit"
        >
          {t('unlock.submit')}
        </Button>
        <Button
          variant="text"
          className={styles.link}
          onClick={() => updateView({ unlockMode: 'recovery' })}
          data-testid="unlock-use-recovery-key"
        >
          {t('unlock.use_recovery_key')}
        </Button>
      </form>
    </div>
  );
}

export default UnlockScreen;
