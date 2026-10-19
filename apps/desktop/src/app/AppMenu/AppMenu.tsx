// The app menu (UI 1.4): the button with three bars in the title bar and its
// menu. Items of later stages are not rendered (src/stage.ts). On macOS the
// native menu bar is used instead and this button is not shown.
//
// Owner: W4.

import { Menu as MenuIcon } from 'lucide-react';
import { useRef, useState } from 'react';
import { IconButton } from '../../components';
import { t } from '../../i18n';
import type { OsKind } from '../../ipc';
import { isVisible } from '../../stage';
import { ContextMenu, type ContextMenuEntry } from '../ContextMenu/ContextMenu';
import { modShortcut } from '../platform';
import { runAppMenuAction } from './actions';
import styles from './AppMenu.module.css';

type Entry = ContextMenuEntry;

function item(id: string, labelKey: string, extra: Partial<Omit<Exclude<Entry, { type: 'separator' }>, 'id'>> = {}): Entry {
  return { id, label: t(labelKey), 'data-testid': id, ...extra };
}

function visible(feature: Parameters<typeof isVisible>[0], entry: Entry): Entry[] {
  return isVisible(feature) ? [entry] : [];
}

/** The app menu entries for the given OS, in the order of UI 1.4. */
export function buildAppMenu(os: OsKind | null): readonly Entry[] {
  const mod = (key: string) => modShortcut(os, key);
  return [
    item('app-menu-vaults', 'menu.vaults', { shortcut: mod('1') }),
    ...visible('menu-sftp', item('app-menu-sftp', 'menu.sftp', { shortcut: mod('2') })),
    item('app-menu-settings', 'menu.settings', { shortcut: mod(',') }),
    { type: 'separator', id: 'separator-1' },
    ...visible(
      'menu-new-local-terminal',
      item('app-menu-new-local-terminal', 'menu.new_local_terminal', { shortcut: mod('L') }),
    ),
    ...visible('menu-new-serial', item('app-menu-new-serial', 'menu.new_serial', { shortcut: 'Ctrl+Alt+S' })),
    item('app-menu-edit', 'menu.edit', {
      children: [
        item('app-menu-edit-undo', 'menu.edit.undo', { shortcut: mod('Z') }),
        item('app-menu-edit-redo', 'menu.edit.redo', { shortcut: os === 'macos' ? 'Cmd+Shift+Z' : 'Ctrl+Y' }),
        { type: 'separator', id: 'edit-separator-1' },
        item('app-menu-edit-cut', 'menu.edit.cut', { shortcut: mod('X') }),
        item('app-menu-edit-copy', 'menu.edit.copy', { shortcut: mod('C') }),
        item('app-menu-edit-paste', 'menu.edit.paste', { shortcut: mod('V') }),
        item('app-menu-edit-select-all', 'menu.edit.select_all', { shortcut: mod('A') }),
      ],
    }),
    item('app-menu-view', 'menu.view', {
      children: [
        item('app-menu-view-zoom-in', 'menu.view.zoom_in'),
        item('app-menu-view-zoom-out', 'menu.view.zoom_out'),
        item('app-menu-view-zoom-reset', 'menu.view.zoom_reset'),
        { type: 'separator', id: 'view-separator-1' },
        item('app-menu-view-fullscreen', 'menu.view.fullscreen'),
        item('app-menu-view-toggle-sidebar', 'menu.view.toggle_sidebar'),
      ],
    }),
    item('app-menu-window', 'menu.window', {
      children: [
        item('app-menu-window-new-window', 'menu.window.new_window'),
        ...(isVisible('menu-window-tabs')
          ? [
              item('app-menu-window-next-tab', 'menu.window.next_tab', { shortcut: 'Alt+→' }),
              item('app-menu-window-previous-tab', 'menu.window.previous_tab', { shortcut: 'Alt+←' }),
              item('app-menu-window-close-tab', 'menu.window.close_tab', { shortcut: mod('Shift+W') }),
            ]
          : []),
      ],
    }),
    item('app-menu-help', 'menu.help', {
      children: [
        item('app-menu-help-documentation', 'menu.help.documentation'),
        item('app-menu-help-report-issue', 'menu.help.report_issue'),
        ...visible('menu-check-updates', item('app-menu-help-check-updates', 'menu.help.check_updates')),
        { type: 'separator', id: 'help-separator-1' },
        item('app-menu-help-about', 'menu.help.about'),
      ],
    }),
    item('app-menu-lock', 'menu.lock'),
    { type: 'separator', id: 'separator-2' },
    item('app-menu-exit', 'menu.exit'),
  ];
}

export interface AppMenuButtonProps {
  os: OsKind | null;
}

export function AppMenuButton({ os }: AppMenuButtonProps) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);

  const toggle = () => {
    if (at !== null) {
      setAt(null);
      return;
    }
    const rect = buttonRef.current?.getBoundingClientRect();
    setAt(rect === undefined ? { x: 0, y: 0 } : { x: rect.left, y: rect.bottom + 4 });
  };

  return (
    <>
      <IconButton
        ref={buttonRef}
        className={styles.button}
        icon={<MenuIcon />}
        aria-label={t('titlebar.menu')}
        aria-haspopup="menu"
        aria-expanded={at !== null}
        onClick={toggle}
        data-keep-edit-target=""
        data-testid="titlebar-menu-button"
      />
      {at === null ? null : (
        <ContextMenu
          items={buildAppMenu(os)}
          at={at}
          anchorRef={buttonRef}
          aria-label={t('titlebar.menu')}
          onClose={() => setAt(null)}
          onAction={runAppMenuAction}
          data-testid="app-menu"
        />
      )}
    </>
  );
}
