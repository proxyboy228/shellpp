// Dialogs of the terminal area: the multi-line paste confirmation (UI 4.3:
// a preview and the line count) and the OSC 8 link confirmation with the full
// target (docs/security.md 12). Rendered by `TerminalHost` only while the
// terminal area is visible, so a lock never leaves them above the unlock
// screen.
//
// Owner: S6.

import { showErrorToast } from '../../app/errors';
import { useStore } from '../../app/store';
import { Button, Modal } from '../../components';
import { t } from '../../i18n';
import { externalOpenUrl } from '../../ipc';
import { linkRequestStore, pasteRequestStore } from './dialogs';
import { pastePreview } from './paste';
import styles from './TerminalHost.module.css';

export function PasteConfirmDialog() {
  const request = useStore(pasteRequestStore);
  const close = () => pasteRequestStore.set(null);
  const paste = () => {
    pasteRequestStore.set(null);
    request?.confirm();
  };
  return (
    <Modal
      open={request !== null}
      title={t('paste.title')}
      onClose={close}
      closeLabel={t('common.close')}
      data-testid="paste-confirm-dialog"
      footer={
        <>
          <Button onClick={close} data-testid="paste-confirm-cancel">
            {t('common.cancel')}
          </Button>
          <Button variant="primary" onClick={paste} data-testid="paste-confirm-paste">
            {t('paste.paste')}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText} data-testid="paste-confirm-lines" data-count={request?.lines ?? 0}>
        {t('paste.lines', { count: request?.lines ?? 0 })}
      </p>
      <pre className={styles.preview} data-testid="paste-confirm-preview">
        {request === null ? '' : pastePreview(request.text)}
      </pre>
    </Modal>
  );
}

/**
 * The text of a link target for the confirmation: everything outside
 * printable ASCII (control, bidi and invisible characters) is shown as
 * `\u{...}`. Targets are normalized URLs (`URL.href`: punycode host,
 * percent-encoded path), so this only guards against surprises.
 */
export function visibleUrl(href: string): string {
  let out = '';
  for (const char of href) {
    const code = char.codePointAt(0) ?? 0;
    out += code >= 0x21 && code <= 0x7e ? char : `\\u{${code.toString(16).toUpperCase()}}`;
  }
  return out;
}

/** The host of a link target as it will be contacted (punycode), or ''. */
function linkHost(href: string): string {
  return URL.canParse(href) ? new URL(href).host : '';
}

export function LinkConfirmDialog() {
  const request = useStore(linkRequestStore);
  const close = () => linkRequestStore.set(null);
  const open = () => {
    linkRequestStore.set(null);
    if (request !== null) {
      externalOpenUrl(request.url, 'terminal').catch(showErrorToast);
    }
  };
  return (
    <Modal
      open={request !== null}
      title={t('links.title')}
      onClose={close}
      closeLabel={t('common.close')}
      data-testid="link-confirm-dialog"
      footer={
        <>
          <Button onClick={close} data-testid="link-confirm-cancel">
            {t('common.cancel')}
          </Button>
          <Button variant="primary" onClick={open} data-testid="link-confirm-open">
            {t('links.open')}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText}>{t('links.text')}</p>
      <p className={styles.dialogText} data-testid="link-confirm-host">
        {t('links.host', { host: visibleUrl(linkHost(request?.url ?? '')) })}
      </p>
      <p className={styles.url} data-testid="link-confirm-url">
        {visibleUrl(request?.url ?? '')}
      </p>
    </Modal>
  );
}
