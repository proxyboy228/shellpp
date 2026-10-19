// Exit and window close confirmation (UI 4.6, 1.4): `exit-confirm-dialog`
// and `window-close-confirm-dialog`. Rendered by the app root in every lock
// state, so that closing a locked window with running sessions still asks.
// `requestAppExit` / `requestWindowClose` (api.ts) open them when the core
// answers `needs_confirmation` (stage 2 plan, decision 17); Confirm calls the
// command again with `confirmed: true`.
//
// Owner: S6.

import { showErrorToast } from '../../app/errors';
import { useStore } from '../../app/store';
import { Button, Modal } from '../../components';
import { t } from '../../i18n';
import { confirmCloseGuard } from './api';
import { closeGuardStore } from './dialogs';
import styles from './TerminalHost.module.css';

export function CloseGuardDialogs() {
  const request = useStore(closeGuardStore);
  const close = () => closeGuardStore.set(null);
  const confirm = () => {
    confirmCloseGuard().catch(showErrorToast);
  };
  const exit = request?.kind === 'exit';
  const testId = exit ? 'exit-confirm-dialog' : 'window-close-confirm-dialog';
  return (
    <Modal
      open={request !== null}
      title={exit ? t('close_guard.exit_title') : t('close_guard.window_title')}
      onClose={close}
      closeLabel={t('common.close')}
      size="small"
      data-testid={testId}
      data-count={request?.count ?? 0}
      footer={
        <>
          <Button onClick={close} data-testid={`${testId}-cancel`}>
            {t('common.cancel')}
          </Button>
          <Button variant="danger" onClick={confirm} data-testid={`${testId}-confirm`}>
            {exit ? t('close_guard.exit_confirm') : t('close_guard.window_confirm')}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText}>
        {exit
          ? t('close_guard.exit_text', { count: request?.count ?? 0 })
          : t('close_guard.window_text', { count: request?.count ?? 0 })}
      </p>
    </Modal>
  );
}
