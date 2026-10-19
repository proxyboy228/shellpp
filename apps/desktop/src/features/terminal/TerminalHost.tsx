// The terminal area of a window (UI 4.1): every terminal tab with its split
// panes, the terminal sidebar and the session overlays. Mounted by the app
// root from the first unlock on and kept mounted while locked (hidden under
// the unlock screen, output keeps arriving; docs/security.md, section 8).
//
// Inactive tab panels stay mounted and laid out (`visibility: hidden`), so
// their terminals keep scrollback, live output and their size.
//
// Owner: S6.

import { useEffect } from 'react';
import { NewTabPage } from '../newtab';
import { linkRequestStore, pasteRequestStore } from './dialogs';
import { LinkConfirmDialog, PasteConfirmDialog } from './TerminalDialogs';
import { PaneTree } from './PaneTree';
import { TerminalSidebar } from './TerminalSidebar';
import { findLeaf, findTab, leavesOf, useSessions, useWorkspace } from './workspace';
import styles from './TerminalHost.module.css';

export interface TerminalHostProps {
  /** False while locked or while the Vaults tab is active: hidden and inert, still mounted. */
  visible: boolean;
}

export function TerminalHost({ visible }: TerminalHostProps) {
  const workspace = useWorkspace();
  const sessions = useSessions();
  const activeTab = workspace.active === 'vaults' ? undefined : findTab(workspace, workspace.active);
  const sidebarShown = visible && workspace.sidebarOpen && activeTab?.kind === 'session';
  // The sidebar acts on the active pane only (UI 4.2): an empty pane has no
  // session, so the sidebar shows "Choose a host in this pane first".
  const sidebarPaneId = sidebarShown ? (activeTab.activePaneId ?? leavesOf(activeTab.root)[0]?.paneId ?? null) : null;
  const sidebarSessionId =
    activeTab === undefined || sidebarPaneId === null ? undefined : (findLeaf(activeTab.root, sidebarPaneId)?.sessionId ?? undefined);

  // Hidden (locked or Vaults): pending paste and link confirmations are dropped.
  useEffect(() => {
    if (!visible) {
      pasteRequestStore.set(null);
      linkRequestStore.set(null);
    }
  }, [visible]);

  return (
    <div
      className={styles.host}
      hidden={!visible}
      inert={!visible}
      data-testid="terminal-host"
      data-active-tab={workspace.active === 'vaults' ? undefined : workspace.active}
    >
      <div className={styles.panels}>
        {workspace.tabs.map((tab) => {
          const active = tab.id === workspace.active;
          return (
            <div
              key={tab.id}
              className={styles.panel}
              hidden={!active}
              inert={!active}
              role="tabpanel"
              data-testid="terminal-tab-panel"
              data-tab-id={tab.id}
            >
              {tab.kind === 'newtab' ? (
                <NewTabPage tabId={tab.id} active={visible && active} />
              ) : (
                <PaneTree tab={tab} sessions={sessions} tabVisible={visible && active} />
              )}
            </div>
          );
        })}
      </div>
      {sidebarShown ? (
        <TerminalSidebar
          tab={workspace.sidebarTab}
          paneId={sidebarPaneId}
          session={sidebarSessionId === undefined ? undefined : sessions.get(sidebarSessionId)}
        />
      ) : null}
      {visible ? (
        <>
          <PasteConfirmDialog />
          <LinkConfirmDialog />
        </>
      ) : null}
    </div>
  );
}
