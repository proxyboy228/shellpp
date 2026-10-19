// Title bar (UI 1.1): a custom frameless title bar. From the left: the app
// menu button, the persistent Vaults tab; on the right: the notification
// center and the window buttons. Elements of later stages are not rendered.
//
// - `minimal` (lock screens, UI 13.2): only the drag area and the window buttons.
// - macOS: the system traffic lights on the left (overlay title bar), no app
//   menu button (native menu bar) and no in-app window buttons.
// - Drag: a press on the empty bar area starts a window drag; a double click
//   toggles maximize (plan, "Routing": `data-tauri-drag-region` is not used).
//
// Owner: W4.

import { ArrowDownToLine, ChevronDown, Copy, Minus, PanelRight, Plus, Square, Vault, X } from 'lucide-react';
import { useEffect, useState, type MouseEvent } from 'react';
import { t } from '../../i18n';
import { getBackend } from '../../ipc';
import { isVisible } from '../../stage';
import { AppMenuButton } from '../AppMenu/AppMenu';
import { NotificationCenter } from '../NotificationCenter/NotificationCenter';
import { isMac } from '../platform';
import { useOs } from '../store';
import styles from './TitleBar.module.css';

export interface TitleBarProps {
  variant: 'full' | 'minimal';
}

/** Elements that take their own clicks: a press on them never drags the window. */
const INTERACTIVE = 'button, a, input, select, textarea, [role="tab"], [role="menu"], [role="dialog"], [data-no-drag]';

function useMaximized(): boolean {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    const controls = getBackend().window;
    const refresh = () => {
      controls
        .isMaximized()
        .then((value) => {
          if (active) {
            setMaximized(value);
          }
        })
        .catch(() => undefined);
    };
    refresh();
    controls
      .onResized(refresh)
      .then((stop) => {
        if (active) {
          unlisten = stop;
        } else {
          stop();
        }
      })
      .catch(() => undefined);
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
  return maximized;
}

function WindowButtons() {
  const maximized = useMaximized();
  const controls = getBackend().window;
  return (
    <div className={styles.windowButtons} data-no-drag="">
      <button
        type="button"
        className={styles.windowButton}
        aria-label={t('titlebar.minimize')}
        onClick={() => void controls.minimize().catch(() => undefined)}
        data-testid="titlebar-window-minimize"
      >
        <Minus size={16} strokeWidth={1.75} aria-hidden="true" />
      </button>
      <button
        type="button"
        className={styles.windowButton}
        aria-label={maximized ? t('titlebar.restore') : t('titlebar.maximize')}
        onClick={() => void controls.toggleMaximize().catch(() => undefined)}
        data-maximized={maximized}
        data-testid="titlebar-window-maximize"
      >
        {maximized ? (
          <Copy size={14} strokeWidth={1.75} aria-hidden="true" />
        ) : (
          <Square size={14} strokeWidth={1.75} aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        className={`${styles.windowButton} ${styles.close}`}
        aria-label={t('titlebar.close')}
        onClick={() => void controls.close().catch(() => undefined)}
        data-testid="titlebar-window-close"
      >
        <X size={16} strokeWidth={1.75} aria-hidden="true" />
      </button>
    </div>
  );
}

function Tabs() {
  return (
    <div role="tablist" aria-label={t('titlebar.tabs')} className={styles.tabs}>
      <button type="button" role="tab" aria-selected="true" className={styles.tab} data-testid="titlebar-tab-vaults">
        <Vault size={16} strokeWidth={1.75} aria-hidden="true" />
        <span>{t('titlebar.vaults')}</span>
      </button>
      {isVisible('topbar-vault-switcher') ? (
        <button
          type="button"
          className={styles.iconTab}
          aria-label={t('titlebar.vault_switcher')}
          data-testid="titlebar-vault-switcher"
        >
          <ChevronDown size={16} strokeWidth={1.75} aria-hidden="true" />
        </button>
      ) : null}
      {isVisible('topbar-sftp-tab') ? (
        <button type="button" role="tab" aria-selected="false" className={styles.tab} data-testid="titlebar-tab-sftp">
          {t('titlebar.sftp')}
        </button>
      ) : null}
      {isVisible('topbar-new-tab') ? (
        <button type="button" className={styles.iconTab} aria-label={t('titlebar.new_tab')} data-testid="titlebar-new-tab">
          <Plus size={16} strokeWidth={1.75} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}

export function TitleBar({ variant }: TitleBarProps) {
  const os = useOs();
  const mac = isMac(os);
  const full = variant === 'full';

  const handleMouseDown = (event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0) {
      return;
    }
    const target = event.target instanceof Element ? event.target : null;
    if (target === null || target.closest(INTERACTIVE) !== null) {
      return;
    }
    const controls = getBackend().window;
    if (event.detail === 2) {
      void controls.toggleMaximize().catch(() => undefined);
    } else {
      void controls.startDragging().catch(() => undefined);
    }
  };

  return (
    <header
      className={styles.bar}
      data-testid="titlebar"
      data-variant={variant}
      data-mac={mac || undefined}
      onMouseDown={handleMouseDown}
    >
      {full && !mac ? <AppMenuButton os={os} /> : null}
      {full ? <Tabs /> : null}
      <div className={styles.dragArea} />
      {full ? (
        <div className={styles.actions}>
          {isVisible('topbar-update-button') ? (
            <button type="button" className={styles.iconTab} aria-label={t('titlebar.update')} data-testid="titlebar-update-button">
              <ArrowDownToLine size={16} strokeWidth={1.75} aria-hidden="true" />
            </button>
          ) : null}
          <NotificationCenter />
          {isVisible('topbar-terminal-sidebar-toggle') ? (
            <button
              type="button"
              className={styles.iconTab}
              aria-label={t('titlebar.terminal_sidebar')}
              data-testid="titlebar-terminal-sidebar-toggle"
            >
              <PanelRight size={16} strokeWidth={1.75} aria-hidden="true" />
            </button>
          ) : null}
        </div>
      ) : null}
      {mac ? null : <WindowButtons />}
    </header>
  );
}

export default TitleBar;
