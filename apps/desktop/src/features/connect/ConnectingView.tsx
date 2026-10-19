// The connection screen of a tab or split pane (UI 4.8): host title and
// address, the current step, Cancel, the prompts (username, password,
// passphrase, keyboard-interactive; "Save" where allowed), the host key
// dialogs (UI 10.2) and the failure state (reason, Retry, Close; the revoked
// warning without Retry). Replies go through `session_prompt_reply`,
// `session_cancel` and `session_reconnect`.
//
// Owner: S7 (skeleton by the lead).

import { RotateCcw, X } from 'lucide-react';
import { useState } from 'react';
import { showErrorToast } from '../../app/errors';
import { Button, StatusDot } from '../../components';
import { sessionCancel, sessionPromptReply, sessionReconnect, type HostKeyDecision, type SessionInfo } from '../../ipc';
import { t } from '../../i18n';
import { HostKeyDialog, HostKeyRevokedWarning } from '../knownhosts/HostKeyDialog';
import { sessionAddress, sessionErrorMessage } from './format';
import { isTextPrompt } from './prompts';
import { SessionPromptForm } from './SessionPromptForm';
import styles from './connect.module.css';

export interface ConnectingViewProps {
  /** A session whose state is `connecting` or `failed`, or that has a `prompt`. */
  session: SessionInfo;
  /** Close the tab or pane (failure state "Close"). */
  onClose(): void;
  /**
   * The pane is the active pane of the visible tab (default true): only then
   * does a prompt or host key dialog take keyboard focus, so input meant for
   * another pane never lands here.
   */
  focused?: boolean;
}

interface HostKeyStepProps {
  session: SessionInfo;
  focused: boolean;
}

function HostKeyStep({ session, focused }: HostKeyStepProps) {
  const [busy, setBusy] = useState(false);
  const prompt = session.prompt;
  if (prompt === undefined || (prompt.kind !== 'host_key_unknown' && prompt.kind !== 'host_key_changed')) {
    return null;
  }
  const decide = (decision: HostKeyDecision) => {
    setBusy(true);
    sessionPromptReply({ session_id: session.session_id, prompt_id: prompt.prompt_id, reply: { kind: 'host_key', decision } })
      .catch(() => {
        // The prompt is gone; the next `sessions:changed` shows the current state.
      })
      .finally(() => setBusy(false));
  };
  return <HostKeyDialog prompt={prompt} hostTitle={session.title} onDecision={decide} busy={busy} focused={focused} />;
}

interface FailureProps {
  session: SessionInfo;
  onClose(): void;
}

function Failure({ session, onClose }: FailureProps) {
  const [retrying, setRetrying] = useState(false);
  const error = session.error;
  const revoked = error?.code === 'host_key_revoked';
  const retry = () => {
    setRetrying(true);
    sessionReconnect(session.session_id)
      .catch(showErrorToast)
      .finally(() => setRetrying(false));
  };
  return (
    <div className={styles.failure} data-code={error?.code} data-reason={error?.reason} data-testid="connect-failed">
      {revoked && session.host_key !== undefined ? (
        <HostKeyRevokedWarning hostTitle={session.title} hostKey={session.host_key} />
      ) : (
        <>
          <p className={styles.failureTitle}>{t('connecting.failed.title')}</p>
          <p className={styles.failureReason} role="alert" data-testid="connect-failed-reason">
            {sessionErrorMessage(error)}
          </p>
        </>
      )}
      <div className={styles.actions}>
        {revoked ? null : (
          <Button variant="primary" icon={<RotateCcw />} loading={retrying} onClick={retry} data-testid="connect-failed-retry">
            {t('connecting.failed.retry')}
          </Button>
        )}
        <Button icon={<X />} onClick={onClose} data-testid="connect-failed-close">
          {t('common.close')}
        </Button>
      </div>
    </div>
  );
}

export function ConnectingView({ session, onClose, focused = true }: ConnectingViewProps) {
  const [cancelling, setCancelling] = useState(false);
  const failed = session.state === 'failed';
  const prompt = session.prompt;
  const address = sessionAddress(session);
  const attempting = session.state === 'connecting' || session.reconnect?.in_progress === true;

  const cancel = () => {
    setCancelling(true);
    sessionCancel(session.session_id)
      .catch(() => {
        // Already finished: the failure or terminal state follows.
      })
      .finally(() => setCancelling(false));
  };

  const hostKeyPrompt = prompt !== undefined && (prompt.kind === 'host_key_unknown' || prompt.kind === 'host_key_changed');
  let body = null;
  if (failed) {
    body = <Failure session={session} onClose={onClose} />;
  } else if (hostKeyPrompt && prompt !== undefined) {
    body = <HostKeyStep key={prompt.prompt_id} session={session} focused={focused} />;
  } else if (isTextPrompt(prompt)) {
    body = (
      <SessionPromptForm
        key={prompt.prompt_id}
        sessionId={session.session_id}
        prompt={prompt}
        hostTitle={session.title}
        focused={focused}
      />
    );
  }

  return (
    <div
      className={styles.view}
      data-session-id={session.session_id}
      data-state={session.state}
      data-step={session.step}
      data-testid="connecting-view"
    >
      <div className={styles.card}>
        <div className={styles.header}>
          <StatusDot
            status={failed ? 'error' : 'connecting'}
            label={failed ? t('connecting.status.failed') : t('connecting.status.connecting')}
            data-testid="connecting-status"
          />
          <h2 className={styles.title} data-testid="connecting-title">
            {session.title}
          </h2>
        </div>
        {address === '' ? null : (
          <p className={styles.address} data-testid="connecting-address">
            {address}
          </p>
        )}
        {!failed && session.step !== undefined ? (
          <p className={styles.step} aria-live="polite" data-testid="connecting-step">
            {t(`connecting.step.${session.step}`)}
          </p>
        ) : null}
        {body}
        {/* A prompt or host key dialog has its own Cancel (the dialog's is its decision, UI 10.2). */}
        {!failed && attempting && !isTextPrompt(prompt) && !hostKeyPrompt ? (
          <div className={styles.actions}>
            <Button loading={cancelling} onClick={cancel} data-testid="connecting-cancel">
              {t('connecting.cancel')}
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}
