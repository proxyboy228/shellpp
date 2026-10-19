// Settings > Security (UI 13.4): auto-lock interval, change master password,
// recovery key Show and Reissue (both after the master password), and
// "Remember on this device" (hidden when no OS keychain is available).
//
// Owner: W4.

import { useState } from 'react';
import { showErrorToast, validationMessage } from '../../app/errors';
import { lockStore, settingsStore, useLockStatus, useSettings } from '../../app/store';
import { Button, Select, Toggle } from '../../components';
import { t } from '../../i18n';
import { rememberDeviceSet, settingsSet, toIpcError, type JsonValue } from '../../ipc';
import { isVisible } from '../../stage';
import { ChangePasswordDialog } from './ChangePasswordDialog';
import { RecoveryKeyDialog } from './RecoveryKeyDialog';
import { RecoveryPasswordDialog, type RecoveryMode } from './RecoveryPasswordDialog';
import { RememberDeviceWarning } from './RememberDeviceWarning';
import styles from './Settings.module.css';

export const AUTO_LOCK_KEY = 'security.auto_lock_minutes';
export type AutoLockOption = '1' | '5' | '15' | '30' | '60' | 'never';
const AUTO_LOCK_OPTIONS: readonly AutoLockOption[] = ['1', '5', '15', '30', '60', 'never'];

/** Select value for the stored setting (default 15 minutes). */
export function autoLockOption(value: JsonValue | undefined): AutoLockOption {
  if (value === null) {
    return 'never';
  }
  const text = String(value ?? 15);
  return (AUTO_LOCK_OPTIONS as readonly string[]).includes(text) ? (text as AutoLockOption) : '15';
}

function autoLockLabel(option: AutoLockOption): string {
  if (option === 'never') {
    return t('security.auto_lock.never');
  }
  return option === '1' ? t('security.auto_lock.minute') : t('security.auto_lock.minutes', { count: option });
}

type Dialog =
  | { kind: 'change-password' }
  | { kind: 'recovery-password'; mode: RecoveryMode }
  | { kind: 'recovery-key'; mode: RecoveryMode; recoveryKey: string }
  | { kind: 'remember-warning' };

export function SecuritySection() {
  const settings = useSettings();
  const lock = useLockStatus();
  const [autoLockError, setAutoLockError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [rememberBusy, setRememberBusy] = useState(false);

  const changeAutoLock = (option: AutoLockOption) => {
    setAutoLockError(null);
    settingsSet({ values: { [AUTO_LOCK_KEY]: option === 'never' ? null : Number(option) } })
      .then((values) => settingsStore.set(values))
      .catch((reason: unknown) => {
        const error = toIpcError(reason);
        const fieldReason = error.fieldReason(AUTO_LOCK_KEY);
        if (fieldReason !== undefined) {
          setAutoLockError(validationMessage(fieldReason));
        } else {
          showErrorToast(error);
        }
      });
  };

  const setRemember = (enabled: boolean) => {
    setRememberBusy(true);
    rememberDeviceSet(enabled)
      // Only the remember fields: a late response must not overwrite a lock
      // state that changed meanwhile (auto-lock); the lock state comes from
      // `lock:changed` and snapshots.
      .then((status) =>
        lockStore.set((current) =>
          current === null
            ? current
            : {
                ...current,
                remember_device_enabled: status.remember_device_enabled,
                remember_device_available: status.remember_device_available,
              },
        ),
      )
      .catch(showErrorToast)
      .finally(() => setRememberBusy(false));
  };

  const closeDialog = () => setDialog(null);

  return (
    <section className={styles.section} aria-labelledby="settings-security-title" data-testid="settings-section-security">
      <h3 id="settings-security-title" className={styles.sectionTitle}>
        {t('settings.security')}
      </h3>

      <div className={styles.block}>
        <Select<AutoLockOption>
          label={t('security.auto_lock')}
          description={t('security.auto_lock_description')}
          value={autoLockOption(settings[AUTO_LOCK_KEY])}
          options={AUTO_LOCK_OPTIONS.map((option) => ({ value: option, label: autoLockLabel(option) }))}
          onValueChange={changeAutoLock}
          error={autoLockError ?? undefined}
          data-testid="security-auto-lock"
        />
      </div>

      <div className={styles.block}>
        <div className={styles.blockText}>
          <h4 className={styles.blockTitle}>{t('security.master_password')}</h4>
          <p className={styles.blockDescription}>{t('security.change_password_description')}</p>
        </div>
        <div className={styles.blockActions}>
          <Button onClick={() => setDialog({ kind: 'change-password' })} data-testid="security-change-password">
            {t('security.change_password')}
          </Button>
        </div>
      </div>

      <div className={styles.block}>
        <div className={styles.blockText}>
          <h4 className={styles.blockTitle}>{t('security.recovery_key')}</h4>
          <p className={styles.blockDescription}>{t('security.recovery_key_description')}</p>
        </div>
        <div className={styles.blockActions}>
          <Button
            onClick={() => setDialog({ kind: 'recovery-password', mode: 'show' })}
            data-testid="security-recovery-show"
          >
            {t('security.recovery_show')}
          </Button>
          <Button
            onClick={() => setDialog({ kind: 'recovery-password', mode: 'reissue' })}
            data-testid="security-recovery-reissue"
          >
            {t('security.recovery_reissue')}
          </Button>
        </div>
      </div>

      {lock?.remember_device_available ? (
        <div className={styles.block}>
          <Toggle
            checked={lock.remember_device_enabled}
            disabled={rememberBusy}
            label={t('security.remember_device')}
            description={t('security.remember_device_description')}
            onCheckedChange={(next) => (next ? setDialog({ kind: 'remember-warning' }) : setRemember(false))}
            data-testid="security-remember-device"
          />
        </div>
      ) : null}

      {isVisible('security-biometric-unlock') ? (
        <div className={styles.block}>
          <Toggle label={t('security.biometric')} data-testid="security-biometric" />
        </div>
      ) : null}

      <ChangePasswordDialog open={dialog?.kind === 'change-password'} onClose={closeDialog} />
      {dialog?.kind === 'recovery-password' ? (
        <RecoveryPasswordDialog
          mode={dialog.mode}
          onCancel={closeDialog}
          onSuccess={(recoveryKey) => setDialog({ kind: 'recovery-key', mode: dialog.mode, recoveryKey })}
        />
      ) : null}
      {dialog?.kind === 'recovery-key' ? (
        <RecoveryKeyDialog mode={dialog.mode} recoveryKey={dialog.recoveryKey} onDone={closeDialog} />
      ) : null}
      <RememberDeviceWarning
        open={dialog?.kind === 'remember-warning'}
        onCancel={closeDialog}
        onConfirm={() => {
          closeDialog();
          setRemember(true);
        }}
      />
    </section>
  );
}

export default SecuritySection;
