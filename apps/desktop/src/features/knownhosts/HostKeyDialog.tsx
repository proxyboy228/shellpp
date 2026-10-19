// Host key dialogs (UI 10.2): first connection (TOFU: Accept and save,
// Connect once, Cancel) and changed key (danger warning, old and new
// fingerprints, only "Replace saved key" continues; no "don't ask again"),
// and the revoked-key warning without a way to continue. Rendered inside the
// connection screen of the tab or pane (features/connect/ConnectingView), as
// an `alertdialog` (stage 2 plan, decision 14), never as a window modal.
//
// Owner: S7 (skeleton by the lead).

import { ShieldAlert, ShieldQuestion, ShieldX } from 'lucide-react';
import { useEffect, useId, useRef } from 'react';
import { Button } from '../../components';
import type { HostKeyChangedPrompt, HostKeyDecision, HostKeyInfo, HostKeyUnknownPrompt } from '../../ipc';
import { t } from '../../i18n';
import { hostKeyAddress } from '../connect/format';
import styles from './knownHosts.module.css';

export interface HostKeyDialogProps {
  prompt: HostKeyUnknownPrompt | HostKeyChangedPrompt;
  /** Host label or address shown in the title. */
  hostTitle: string;
  onDecision(decision: HostKeyDecision): void;
  /** Buttons are inactive while a decision is being sent. */
  busy?: boolean;
  /** The pane is active and visible (default true): only then does the default button take focus. */
  focused?: boolean;
}

interface KeyRowsProps {
  keyType: string;
  fingerprint: string;
  testIdPrefix: string;
  label?: string;
  /** The address row first, in the same list (`<prefix>address`). */
  address?: string;
}

function KeyRows({ keyType, fingerprint, testIdPrefix, label, address }: KeyRowsProps) {
  return (
    <div className={styles.keyBlock}>
      {label === undefined ? null : <p className={styles.keyBlockTitle}>{label}</p>}
      <dl className={styles.facts}>
        {address === undefined ? null : (
          <>
            <dt>{t('host_key.address')}</dt>
            <dd data-testid={`${testIdPrefix}address`}>{address}</dd>
          </>
        )}
        <dt>{t('host_key.key_type')}</dt>
        <dd data-testid={`${testIdPrefix}key-type`}>{keyType}</dd>
        <dt>{t('host_key.fingerprint')}</dt>
        <dd className={styles.fingerprint} data-testid={`${testIdPrefix}fingerprint`}>
          {fingerprint}
        </dd>
      </dl>
    </div>
  );
}

export function HostKeyDialog({ prompt, hostTitle, onDecision, busy = false, focused = true }: HostKeyDialogProps) {
  const titleId = useId();
  const textId = useId();
  const defaultRef = useRef<HTMLButtonElement>(null);
  const changed = prompt.kind === 'host_key_changed';
  const { key } = prompt;

  // The default button takes focus: Accept and save for a first connection,
  // Cancel for a changed key (continuing must be an explicit choice); only in
  // the active pane of the visible tab, never away from another pane.
  useEffect(() => {
    if (focused) {
      defaultRef.current?.focus();
    }
  }, [prompt, focused]);

  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={textId}
      className={styles.hostKeyDialog}
      data-kind={changed ? 'changed' : 'unknown'}
      data-testid="host-key-dialog"
    >
      <div className={styles.hostKeyHeader}>
        <span className={styles.hostKeyIcon} aria-hidden="true">
          {changed ? <ShieldAlert size={20} strokeWidth={1.75} /> : <ShieldQuestion size={20} strokeWidth={1.75} />}
        </span>
        <h3 id={titleId} className={styles.hostKeyTitle}>
          {changed ? t('host_key.changed.title') : t('host_key.unknown.title')}
        </h3>
      </div>
      {changed ? (
        <p id={textId} className={styles.hostKeyWarning} data-testid="host-key-dialog-warning">
          {t('host_key.changed.warning', { host: hostTitle })}
        </p>
      ) : (
        <p id={textId} className={styles.hostKeyText}>
          {t('host_key.unknown.text', { host: hostTitle })}
        </p>
      )}
      {changed ? (
        <>
          <dl className={styles.facts}>
            <dt>{t('host_key.address')}</dt>
            <dd data-testid="host-key-dialog-address">{hostKeyAddress(key.address, key.port)}</dd>
          </dl>
          <KeyRows
            label={t('host_key.changed.old_key')}
            keyType={prompt.old_key_type}
            fingerprint={prompt.old_fingerprint}
            testIdPrefix="host-key-dialog-old-"
          />
          <KeyRows
            label={t('host_key.changed.new_key')}
            keyType={key.key_type}
            fingerprint={key.fingerprint}
            testIdPrefix="host-key-dialog-"
          />
          <p className={styles.hostKeyHint}>{t('host_key.changed.hint')}</p>
        </>
      ) : (
        <KeyRows
          address={hostKeyAddress(key.address, key.port)}
          keyType={key.key_type}
          fingerprint={key.fingerprint}
          testIdPrefix="host-key-dialog-"
        />
      )}
      <div className={styles.hostKeyActions}>
        {changed ? (
          <>
            <Button variant="danger" disabled={busy} onClick={() => onDecision('replace')} data-testid="host-key-dialog-replace">
              {t('host_key.changed.replace')}
            </Button>
            <Button ref={defaultRef} disabled={busy} onClick={() => onDecision('cancel')} data-testid="host-key-dialog-cancel">
              {t('common.cancel')}
            </Button>
          </>
        ) : (
          <>
            <Button
              ref={defaultRef}
              variant="primary"
              disabled={busy}
              onClick={() => onDecision('accept_save')}
              data-testid="host-key-dialog-accept"
            >
              {t('host_key.unknown.accept')}
            </Button>
            <Button disabled={busy} onClick={() => onDecision('connect_once')} data-testid="host-key-dialog-once">
              {t('host_key.unknown.once')}
            </Button>
            <Button disabled={busy} onClick={() => onDecision('cancel')} data-testid="host-key-dialog-cancel">
              {t('common.cancel')}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

export interface HostKeyRevokedWarningProps {
  hostTitle: string;
  hostKey: HostKeyInfo;
}

/** The presented key is a revoked known host entry: blocked, no button to continue (UI 10.2). */
export function HostKeyRevokedWarning({ hostTitle, hostKey }: HostKeyRevokedWarningProps) {
  const titleId = useId();
  return (
    <div
      role="alert"
      aria-labelledby={titleId}
      className={styles.hostKeyDialog}
      data-kind="revoked"
      data-key-type={hostKey.key_type}
      data-testid="host-key-revoked"
    >
      <div className={styles.hostKeyHeader}>
        <span className={styles.hostKeyIcon} aria-hidden="true">
          <ShieldX size={20} strokeWidth={1.75} />
        </span>
        <h3 id={titleId} className={styles.hostKeyTitle}>
          {t('host_key.revoked.title')}
        </h3>
      </div>
      <p className={styles.hostKeyWarning}>{t('host_key.revoked.warning', { host: hostTitle })}</p>
      <KeyRows
        address={hostKeyAddress(hostKey.address, hostKey.port)}
        keyType={hostKey.key_type}
        fingerprint={hostKey.fingerprint}
        testIdPrefix="host-key-revoked-"
      />
      <p className={styles.hostKeyHint}>{t('host_key.revoked.hint')}</p>
    </div>
  );
}
