// Session end overlays over the terminal (UI 4.6; D18): the output stays
// visible underneath.
//
// - `ended` (non-zero exit status or a signal): "Session ended with code N",
//   Reconnect and Close; no automatic attempts.
// - `dropped` (transport error, keepalive timeout, closed without status): the
//   reason, a countdown to the next automatic attempt with Cancel (stops the
//   attempts), Reconnect now and Close; while locked the attempts wait.
// - an attempt in progress (automatic, Reconnect now, Reconnect after an
//   end): "Reconnecting" with the current step, Cancel (stops the attempt
//   and the automatic attempts, UI 4.6, 4.8) and Close.
//
// Owner: S6.

import { RotateCw } from 'lucide-react';
import { showErrorToast } from '../../app/errors';
import { Button } from '../../components';
import { t } from '../../i18n';
import { sessionCancel, sessionReconnect, type SessionError, type SessionInfo } from '../../ipc';
import { useClock } from './clock';
import { reconnectDeadline } from './sessionsSync';
import styles from './TerminalHost.module.css';

/** The text of a session error: the SSH reason when there is one, else the error code. */
function sessionErrorText(error: SessionError | undefined): string {
  if (error === undefined) {
    return t('errors.ssh_error');
  }
  if (error.code === 'ssh_error' && error.reason !== undefined) {
    return t(`ssh_reason.${error.reason}`);
  }
  return t(`errors.${error.code}`);
}

function reconnect(session: SessionInfo): void {
  sessionReconnect(session.session_id).catch(showErrorToast);
}

export function SessionEndedOverlay({ session, onClose }: { session: SessionInfo; onClose(): void }) {
  const code = session.exit_status ?? 0;
  return (
    <div className={styles.overlay} role="status" data-testid="session-ended" data-exit-status={code}>
      <div className={styles.overlayCard}>
        <p className={styles.overlayMessage} data-testid="session-ended-message">
          {t('session_end.ended', { code })}
        </p>
        <div className={styles.overlayActions}>
          <Button variant="primary" onClick={() => reconnect(session)} data-testid="session-ended-reconnect">
            {t('session_end.reconnect')}
          </Button>
          <Button onClick={onClose} data-testid="session-ended-close">
            {t('session_end.close')}
          </Button>
        </div>
      </div>
    </div>
  );
}

function Countdown({ sessionId }: { sessionId: string }) {
  const now = useClock();
  const deadline = reconnectDeadline(sessionId);
  const seconds = deadline === undefined ? 0 : Math.max(0, Math.ceil((deadline - now) / 1000));
  return (
    <p className={styles.overlayDetail} data-testid="session-dropped-countdown" data-seconds={seconds}>
      {t('session_end.countdown', { seconds })}
    </p>
  );
}

export function SessionDroppedOverlay({ session, onClose }: { session: SessionInfo; onClose(): void }) {
  const attempts = session.reconnect;
  const scheduled = attempts !== undefined && !attempts.in_progress;
  return (
    <div
      className={styles.overlay}
      role="status"
      data-testid="session-dropped"
      data-code={session.error?.code}
      data-reason={session.error?.reason}
    >
      <div className={styles.overlayCard}>
        <p className={styles.overlayMessage} data-testid="session-dropped-message">
          {sessionErrorText(session.error)}
        </p>
        {scheduled && attempts.waiting_for_unlock ? (
          <p className={styles.overlayDetail}>{t('session_end.waiting_for_unlock')}</p>
        ) : null}
        {scheduled && !attempts.waiting_for_unlock ? <Countdown sessionId={session.session_id} /> : null}
        <div className={styles.overlayActions}>
          {scheduled ? (
            <Button
              onClick={() => sessionCancel(session.session_id).catch(showErrorToast)}
              data-testid="session-dropped-cancel"
            >
              {t('session_end.cancel')}
            </Button>
          ) : null}
          <Button variant="primary" onClick={() => reconnect(session)} data-testid="session-dropped-reconnect">
            {scheduled ? t('session_end.reconnect_now') : t('session_end.reconnect')}
          </Button>
          <Button onClick={onClose} data-testid="session-dropped-close">
            {t('session_end.close')}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function SessionReconnectingOverlay({ session, onClose }: { session: SessionInfo; onClose(): void }) {
  return (
    <div className={styles.overlay} role="status" data-testid="session-reconnecting" data-step={session.step}>
      <div className={styles.overlayCard}>
        <p className={styles.overlayMessage}>
          <RotateCw className={styles.spin} size={16} strokeWidth={1.75} aria-hidden="true" />
          {session.reconnect === undefined
            ? t('session_end.reconnecting')
            : t('session_end.reconnecting_attempt', { attempt: session.reconnect.attempt })}
        </p>
        {session.step === undefined ? null : (
          <p className={styles.overlayDetail} aria-live="polite" data-testid="session-reconnecting-step">
            {t(`connecting.step.${session.step}`)}
          </p>
        )}
        <div className={styles.overlayActions}>
          <Button
            onClick={() => sessionCancel(session.session_id).catch(showErrorToast)}
            data-testid="session-reconnecting-cancel"
          >
            {t('session_end.cancel')}
          </Button>
          <Button onClick={onClose} data-testid="session-reconnecting-close">
            {t('session_end.close')}
          </Button>
        </div>
      </div>
    </div>
  );
}
