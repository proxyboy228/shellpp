// SSH key credentials of the host panel (UI 3.1): the linked key field (key
// label and type, fingerprint on hover, Unlink, a "missing" state), the
// inherited key ("from group X") and the "+" button with its menu. The "+"
// menu offers Key (identity, certificate and FIDO2 key are stage 3); a type
// that is already linked leaves the menu, so the button is hidden while a key
// is linked. Key opens the "+ -> Key" dialog.
//
// Owner: S7.

import { KeyRound, Link2Off, Plus } from 'lucide-react';
import { useRef, useState } from 'react';
import { Button, IconButton } from '../../../components';
import type { KeyView, Uuid } from '../../../ipc';
import { t } from '../../../i18n';
import { KeyAddDialog } from '../../connect/KeyAddDialog';
import { useKeys } from '../../connect/useKeys';
import { handleMenuKeys } from '../../hosts/ActionBar';
import hostsStyles from '../../hosts/HostsScreen.module.css';
import { Popover } from '../Popover';
import styles from '../panels.module.css';

export interface KeyFieldProps {
  vaultId: Uuid;
  /** The host's own linked key (`''`: none). */
  keyId: string;
  /** A key inherited from a group ("from group X"). */
  inherited?: { keyId: Uuid; groupName: string };
  onLink: (keyId: Uuid) => void;
  onUnlink: () => void;
  /** Test id prefix, `host-panel`. */
  testIdPrefix: string;
}

export function KeyField({ vaultId, keyId, inherited, onLink, onUnlink, testIdPrefix: TID }: KeyFieldProps) {
  const keys = useKeys();
  const [menuOpen, setMenuOpen] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const addRef = useRef<HTMLButtonElement>(null);

  const own = keyId !== '';
  const shownId = own ? keyId : inherited?.keyId;
  const key: KeyView | undefined = shownId === undefined ? undefined : keys?.find((candidate) => candidate.record_id === shownId);
  const missing = shownId !== undefined && keys !== null && key === undefined;

  return (
    <div className={styles.field}>
      {shownId !== undefined ? (
        <div
          className={styles.linkedKey}
          title={key?.fingerprint}
          data-key-id={shownId}
          data-inherited={own ? undefined : 'true'}
          data-testid={`${TID}-ssh-linked-key`}
        >
          <span className={styles.linkedKeyIcon} aria-hidden="true">
            <KeyRound size={16} strokeWidth={1.75} />
          </span>
          <span className={styles.linkedKeyText}>
            {missing ? (
              <span className={styles.linkedKeyMissing} data-testid={`${TID}-ssh-linked-key-missing`}>
                {t('host_panel.key.missing')}
              </span>
            ) : (
              <>
                <span className={styles.linkedKeyLabel} data-testid={`${TID}-ssh-linked-key-label`}>
                  {key?.label ?? ''}
                </span>
                <span className={styles.linkedKeyType} data-testid={`${TID}-ssh-linked-key-type`}>
                  {key?.algorithm ?? ''}
                </span>
              </>
            )}
            {!own && inherited !== undefined ? (
              <span className={styles.linkedKeyType} data-testid={`${TID}-ssh-linked-key-inherited`}>
                {t('host_panel.key.inherited', { group: inherited.groupName })}
              </span>
            ) : null}
          </span>
          {own ? (
            <IconButton
              size="sm"
              icon={<Link2Off />}
              aria-label={t('host_panel.key.unlink')}
              title={t('host_panel.key.unlink')}
              onClick={onUnlink}
              data-testid={`${TID}-ssh-linked-key-unlink`}
            />
          ) : null}
        </div>
      ) : null}
      {own ? null : (
        <div className={hostsStyles.anchor}>
          <Button
            ref={addRef}
            variant="text"
            icon={<Plus />}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
            data-testid={`${TID}-ssh-credentials-add`}
          >
            {t('host_panel.credentials.add')}
          </Button>
          {menuOpen ? (
            <Popover
              anchorRef={addRef}
              onClose={() => setMenuOpen(false)}
              placement="below-start"
              role="menu"
              aria-label={t('host_panel.credentials.menu')}
              data-testid={`${TID}-credentials-menu`}
            >
              <div className={hostsStyles.menuItems} onKeyDown={handleMenuKeys}>
                <button
                  type="button"
                  role="menuitem"
                  className={hostsStyles.menuItem}
                  autoFocus
                  onClick={() => {
                    setMenuOpen(false);
                    setDialogOpen(true);
                  }}
                  data-testid={`${TID}-credentials-menu-key`}
                >
                  <span className={hostsStyles.menuCheck} aria-hidden="true">
                    <KeyRound size={16} strokeWidth={1.75} />
                  </span>
                  <span className={hostsStyles.menuLabel}>{t('host_panel.credentials.key')}</span>
                </button>
                {/* `host-credentials-identity` (stage 3): identity, certificate and FIDO2 key are hidden. */}
              </div>
            </Popover>
          ) : null}
        </div>
      )}
      {dialogOpen ? (
        <KeyAddDialog vaultId={vaultId} onLinked={(linked) => onLink(linked.record_id)} onClose={() => setDialogOpen(false)} />
      ) : null}
    </div>
  );
}
