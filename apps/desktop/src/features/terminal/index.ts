// Terminal feature (UI 4.1-4.4, 4.6, 4.7; stage 2 plan, section 6.2):
// workspace of tabs and splits, terminal views (xterm.js), terminal sidebar,
// session end overlays, paste confirmation, history capture, close
// confirmation, last session. The public surface for the app shell and S7.
//
// Owner: S6.

export {
  activateNextTab,
  activatePane,
  activatePreviousTab,
  activateTab,
  activateTabByNumber,
  applySnapshotSessions,
  closeActiveTab,
  closePane,
  closeTab,
  duplicateTab,
  focusPaneInDirection,
  handleWindowIntent,
  openLocalTerminal,
  openNewTab,
  openSession,
  openTerminalSidebar,
  requestAppExit,
  requestWindowClose,
  restoreLastSession,
  setQuickConnectOpener,
  splitPane,
  toggleTerminalSidebar,
  useLastSessionAvailable,
} from './api';
export type { OpenOptions, OpenResult } from './api';
export { CloseGuardDialogs } from './CloseGuard';
export { mergeSession, startSessionsSync, watchSession } from './sessionsSync';
export { installTerminalShortcuts, isAppShortcut, terminalHasFocus } from './shortcuts';
export { TerminalHost } from './TerminalHost';
export type { TerminalHostProps } from './TerminalHost';
export { NewTabButton, SessionTabs, TerminalSidebarToggle } from './TitleBarTabs';
export {
  INITIAL_WORKSPACE,
  resetWorkspace,
  sessionsStore,
  summarizeTabs,
  useSessions,
  useTabSummaries,
  useWorkspace,
  workspaceStore,
} from './workspace';
export type { PaneId, PaneNode, SidebarTab, TabId, TabSummary, TerminalTab, WorkspaceState } from './workspace';
