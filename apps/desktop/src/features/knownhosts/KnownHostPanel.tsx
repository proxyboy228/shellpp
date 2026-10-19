// Details of a known host entry (UI 10.1) in the details panel: address,
// port, key type, SHA256 fingerprint with copy, date added, the saved hosts
// with this address (a click opens the host on the Hosts screen) and Remove.
//
// Owner: S7.

import { Copy, Trash2 } from 'lucide-react';
import { useRef } from 'react';
import { DetailsPanel } from '../../app/DetailsPanel/DetailsPanel';
import type { ContextMenuEntry } from '../../app/ContextMenu/ContextMenu';
import { Button, IconButton } from '../../components';
import type { KnownHostView, Uuid } from '../../ipc';
import { t } from '../../i18n';
import { usePanelRootAttributes } from '../panels/panelAttributes';
import { knownHostTitle } from './model';
import styles from './knownHosts.module.css';

const TID = 'known-host-panel';

export interface KnownHostPanelProps {
  entry: KnownHostView;
  /** Titles of saved hosts by id (label, else address). */
  hostTitles: ReadonlyMap<Uuid, string>;
  onCopyFingerprint: (entry: KnownHostView) => void;
  onRemove: (entry: KnownHostView) => void;
  onOpenHost: (recordId: Uuid) => void;
  onClose: () => void;
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function KnownHostPanel({ entry, hostTitles, onCopyFingerprint, onRemove, onOpenHost, onClose }: KnownHostPanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  usePanelRootAttributes(containerRef, TID, { 'data-record-id': entry.record_id });

  const hosts = entry.host_ids
    .map((id) => ({ id, title: hostTitles.get(id) }))
    .filter((host): host is { id: Uuid; title: string } => host.title !== undefined);

  const menuItems: ContextMenuEntry[] = [
    {
      id: 'copy-fingerprint',
      label: t('known_hosts.menu.copy_fingerprint'),
      icon: Copy,
      onSelect: () => onCopyFingerprint(entry),
      'data-testid': `${TID}-menu-copy-fingerprint`,
    },
    { type: 'separator', id: 'separator' },
    {
      id: 'remove',
      label: t('known_hosts.menu.remove'),
      icon: Trash2,
      danger: true,
      onSelect: () => onRemove(entry),
      'data-testid': `${TID}-menu-remove`,
    },
  ];

  return (
    <div ref={containerRef} className={styles.panelSlot}>
      <DetailsPanel
        title={t('known_hosts.panel.title')}
        menuItems={menuItems}
        onCollapse={onClose}
        data-testid={TID}
        footer={
          <Button variant="danger" icon={<Trash2 />} fullWidth onClick={() => onRemove(entry)} data-testid={`${TID}-remove`}>
            {t('known_hosts.panel.remove')}
          </Button>
        }
      >
        <div className={styles.panelBody}>
          <dl className={styles.panelRows}>
            <div className={styles.panelRow}>
              <dt className={styles.panelLabel}>{t('known_hosts.panel.address')}</dt>
              <dd className={styles.panelValue} data-testid={`${TID}-address`}>
                {knownHostTitle(entry)}
              </dd>
            </div>
            <div className={styles.panelRow}>
              <dt className={styles.panelLabel}>{t('known_hosts.panel.port')}</dt>
              <dd className={styles.panelValue} data-testid={`${TID}-port`}>
                {entry.hashed ? t('known_hosts.panel.port_hashed') : String(entry.port ?? 22)}
              </dd>
            </div>
            <div className={styles.panelRow}>
              <dt className={styles.panelLabel}>{t('known_hosts.panel.key_type')}</dt>
              <dd className={styles.panelValue} data-testid={`${TID}-key-type`}>
                {entry.key_type}
                {entry.revoked ? (
                  <>
                    {' '}
                    <span className={styles.revokedBadge}>{t('known_hosts.revoked')}</span>
                  </>
                ) : null}
              </dd>
            </div>
            <div className={styles.panelRow}>
              <dt className={styles.panelLabel}>{t('known_hosts.panel.fingerprint')}</dt>
              <dd className={styles.panelValueRow}>
                <span className={`${styles.panelValue} ${styles.fingerprint}`} data-testid={`${TID}-fingerprint`}>
                  {entry.fingerprint}
                </span>
                <IconButton
                  size="sm"
                  icon={<Copy />}
                  aria-label={t('known_hosts.panel.copy_fingerprint')}
                  title={t('known_hosts.panel.copy_fingerprint')}
                  onClick={() => onCopyFingerprint(entry)}
                  data-testid={`${TID}-fingerprint-copy`}
                />
              </dd>
            </div>
            <div className={styles.panelRow}>
              <dt className={styles.panelLabel}>{t('known_hosts.panel.added')}</dt>
              <dd className={styles.panelValue} data-testid={`${TID}-added`}>
                <time dateTime={entry.created_at}>{formatDate(entry.created_at)}</time>
              </dd>
            </div>
            {entry.comment !== undefined && entry.comment !== '' ? (
              <div className={styles.panelRow}>
                <dt className={styles.panelLabel}>{t('known_hosts.panel.comment')}</dt>
                <dd className={styles.panelValue}>{entry.comment}</dd>
              </div>
            ) : null}
            <div className={styles.panelRow}>
              <dt className={styles.panelLabel}>{t('known_hosts.panel.hosts')}</dt>
              <dd className={styles.panelValue}>
                {hosts.length === 0 ? (
                  <p className={styles.muted} data-testid={`${TID}-hosts-empty`}>
                    {t('known_hosts.panel.hosts_empty')}
                  </p>
                ) : (
                  <ul className={styles.hostLinks} data-testid={`${TID}-hosts`}>
                    {hosts.map((host) => (
                      <li key={host.id}>
                        <button
                          type="button"
                          className={styles.hostLink}
                          onClick={() => onOpenHost(host.id)}
                          data-record-id={host.id}
                          data-testid={`${TID}-host`}
                        >
                          {host.title}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </dd>
            </div>
          </dl>
        </div>
      </DetailsPanel>
    </div>
  );
}
