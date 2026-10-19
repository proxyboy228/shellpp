// Title bar parts of the terminal (UI 1.1): the session tabs, the "+" button
// and the terminal sidebar toggle. Rendered by the app shell's TitleBar.
//
// A session tab shows the host OS icon and the title (the custom title, else
// the session title); on the active tab and on hover the close button
// replaces the OS icon; the active tab carries the `--accent` bar (D25); the
// session type icon sits at the right. Right click opens the tab menu of
// UI 4.4 (multiplayer and broadcast input are later stages); Rename edits
// the title inline (Enter saves, Escape cancels; the host is not renamed).
// Middle click closes a tab. Closing follows the neighbor rule of UI 4.6.
//
// Owner: S6.

import { Columns2, Copy, PanelRight, Pencil, Plus, Rows2, SquareTerminal, X, AppWindow } from 'lucide-react';
import { useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react';
import { ContextMenu, type ContextMenuEntry } from '../../app/ContextMenu/ContextMenu';
import { modShortcut } from '../../app/platform';
import { useOs, useStore } from '../../app/store';
import { OsIcon } from '../../components';
import { t } from '../../i18n';
import { isVisible } from '../../stage';
import {
  activateTab,
  closeTab,
  duplicateTab,
  openNewTab,
  setTabTitle,
  splitPane,
  startRenameTab,
  toggleTerminalSidebar,
} from './api';
import { renamingTabStore } from './dialogs';
import { useTabSummaries, useWorkspace, type TabSummary } from './workspace';
import styles from './TitleBarTabs.module.css';

function RenameInput({ tab }: { tab: TabSummary }) {
  const [value, setValue] = useState(tab.title);
  const inputRef = useRef<HTMLInputElement>(null);
  const done = useRef(false);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const finish = (save: boolean) => {
    if (done.current) {
      return;
    }
    done.current = true;
    if (save) {
      setTabTitle(tab.id, value);
    }
    renamingTabStore.set(null);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    event.stopPropagation();
    if (event.key === 'Enter') {
      event.preventDefault();
      finish(true);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      finish(false);
    }
  };

  return (
    <input
      ref={inputRef}
      className={styles.renameInput}
      value={value}
      maxLength={256}
      aria-label={t('tabs.rename_label')}
      onChange={(event) => setValue(event.target.value)}
      onKeyDown={onKeyDown}
      onBlur={() => finish(true)}
      onClick={(event) => event.stopPropagation()}
      data-testid="titlebar-session-tab-rename-input"
    />
  );
}

function tabMenu(tab: TabSummary, shortcut: (key: string) => string): ContextMenuEntry[] {
  const session = tab.kind === 'session' && tab.sessionIds.length > 0;
  const entries: ContextMenuEntry[] = [
    {
      id: 'duplicate',
      label: t('tabs.menu.duplicate'),
      icon: Copy,
      disabled: !session,
      onSelect: () => void duplicateTab(tab.id),
      'data-testid': 'session-tab-menu-duplicate',
    },
    {
      id: 'duplicate-window',
      label: t('tabs.menu.duplicate_window'),
      icon: AppWindow,
      disabled: !session,
      onSelect: () => void duplicateTab(tab.id, { newWindow: true }),
      'data-testid': 'session-tab-menu-duplicate-window',
    },
  ];
  if (isVisible('session-multiplayer')) {
    entries.push({ id: 'multiplayer', label: t('tabs.menu.multiplayer'), 'data-testid': 'session-tab-menu-multiplayer' });
  }
  entries.push(
    { type: 'separator', id: 'separator-rename' },
    {
      id: 'rename',
      label: t('tabs.menu.rename'),
      icon: Pencil,
      // A new-tab page is named "New Tab" until a host is chosen (UI 6).
      disabled: tab.kind !== 'session',
      onSelect: () => startRenameTab(tab.id),
      'data-testid': 'session-tab-menu-rename',
    },
    { type: 'separator', id: 'separator-split' },
    {
      id: 'split-horizontal',
      label: t('tabs.menu.split_horizontal'),
      icon: Rows2,
      shortcut: shortcut('Shift+E'),
      disabled: tab.kind !== 'session',
      onSelect: () => void splitPane('column', tab.id),
      'data-testid': 'session-tab-menu-split-horizontal',
    },
    {
      id: 'split-vertical',
      label: t('tabs.menu.split_vertical'),
      icon: Columns2,
      shortcut: shortcut('Shift+D'),
      disabled: tab.kind !== 'session',
      onSelect: () => void splitPane('row', tab.id),
      'data-testid': 'session-tab-menu-split-vertical',
    },
  );
  if (isVisible('session-broadcast') && tab.sessionIds.length > 1) {
    entries.push({ id: 'broadcast', label: t('tabs.menu.broadcast'), 'data-testid': 'session-tab-menu-broadcast' });
  }
  entries.push(
    { type: 'separator', id: 'separator-close' },
    {
      id: 'close',
      label: t('tabs.menu.close'),
      icon: X,
      shortcut: shortcut('Shift+W'),
      onSelect: () => closeTab(tab.id),
      'data-testid': 'session-tab-menu-close',
    },
  );
  return entries;
}

function SessionTab({ tab, active, renaming, onMenu }: { tab: TabSummary; active: boolean; renaming: boolean; onMenu(event: MouseEvent, tab: TabSummary): void }) {
  const title = tab.kind === 'newtab' ? t('tabs.new_tab') : tab.title;

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) {
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activateTab(tab.id);
    } else if (event.key === 'F2' && tab.kind === 'session') {
      event.preventDefault();
      startRenameTab(tab.id);
    } else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
      event.preventDefault();
      const rect = event.currentTarget.getBoundingClientRect();
      onMenu({ clientX: rect.left, clientY: rect.bottom, preventDefault: () => undefined } as unknown as MouseEvent, tab);
    }
  };

  return (
    <div
      role="tab"
      tabIndex={0}
      aria-selected={active}
      className={styles.tab}
      title={title}
      onClick={() => activateTab(tab.id)}
      onMouseDown={(event) => {
        if (event.button === 1) {
          // Middle click closes the tab.
          event.preventDefault();
        }
      }}
      onAuxClick={(event) => {
        if (event.button === 1) {
          closeTab(tab.id);
        }
      }}
      onContextMenu={(event) => onMenu(event, tab)}
      onKeyDown={onKeyDown}
      data-testid="titlebar-session-tab"
      data-tab-id={tab.id}
      data-kind={tab.kind}
      data-state={tab.state}
    >
      <span className={styles.lead}>
        {active ? null : (
          <OsIcon os={tab.os} className={styles.osIcon} data-testid="titlebar-session-tab-os-icon" />
        )}
        <button
          type="button"
          className={styles.close}
          data-visible={active || undefined}
          aria-label={t('tabs.close', { title })}
          tabIndex={-1}
          onClick={(event) => {
            event.stopPropagation();
            closeTab(tab.id);
          }}
          data-testid="titlebar-session-tab-close"
        >
          <X size={14} strokeWidth={1.75} aria-hidden="true" />
        </button>
      </span>
      {renaming ? (
        <RenameInput tab={tab} />
      ) : (
        <span className={styles.title} data-testid="titlebar-session-tab-title">
          {title}
        </span>
      )}
      {tab.kind === 'session' ? (
        <span className={styles.kind} title={t('tabs.kind_terminal')} data-testid="titlebar-session-tab-kind" data-kind="terminal">
          <SquareTerminal size={14} strokeWidth={1.75} aria-hidden="true" />
        </span>
      ) : null}
    </div>
  );
}

/** The session tabs of this window, in order (`titlebar-session-tab`). */
export function SessionTabs() {
  const tabs = useTabSummaries();
  const { active } = useWorkspace();
  const renaming = useStore(renamingTabStore);
  const os = useOs();
  const [menu, setMenu] = useState<{ tab: TabSummary; at: { x: number; y: number } } | null>(null);

  const openMenu = (event: MouseEvent, tab: TabSummary) => {
    event.preventDefault();
    setMenu({ tab, at: { x: event.clientX, y: event.clientY } });
  };

  return (
    <>
      {tabs.map((tab) => (
        <SessionTab key={tab.id} tab={tab} active={active === tab.id} renaming={renaming === tab.id} onMenu={openMenu} />
      ))}
      {menu === null ? null : (
        <ContextMenu
          items={tabMenu(menu.tab, (key) => modShortcut(os, key))}
          at={menu.at}
          onClose={() => setMenu(null)}
          aria-label={t('tabs.menu.label')}
          data-testid="session-tab-menu"
        />
      )}
    </>
  );
}

/** "+": a new tab with the new tab page (UI 6). */
export function NewTabButton() {
  return (
    <button
      type="button"
      className={styles.iconButton}
      aria-label={t('titlebar.new_tab')}
      onClick={() => openNewTab()}
      data-testid="titlebar-new-tab"
    >
      <Plus size={16} strokeWidth={1.75} aria-hidden="true" />
    </button>
  );
}

/** Shown only while a session tab is active (UI 1.1). */
export function TerminalSidebarToggle() {
  const workspace = useWorkspace();
  const activeTab = workspace.tabs.find((tab) => tab.id === workspace.active);
  if (activeTab === undefined || activeTab.kind !== 'session') {
    return null;
  }
  return (
    <button
      type="button"
      className={styles.iconButton}
      aria-label={t('titlebar.terminal_sidebar')}
      aria-pressed={workspace.sidebarOpen}
      onClick={toggleTerminalSidebar}
      data-testid="titlebar-terminal-sidebar-toggle"
    >
      <PanelRight size={16} strokeWidth={1.75} aria-hidden="true" />
    </button>
  );
}
