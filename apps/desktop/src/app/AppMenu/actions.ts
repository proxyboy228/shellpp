// Actions of the app menu (UI 1.4). Item ids are the menu test ids
// (`app-menu-settings`, ...); the macOS native menu bar sends the same ids in
// `menu:action` events (plan, decision 20), so both run these functions.
//
// Owner: lead (stage 2 wiring to features/terminal).

import {
  activateNextTab,
  activatePreviousTab,
  activateTab,
  closeActiveTab,
  openLocalTerminal,
  requestAppExit,
} from '../../features/terminal';
import { externalOpenUrl, getBackend, lock, settingsSet, windowOpen } from '../../ipc';
import { MAX_ZOOM, MIN_ZOOM, normalizeZoom, UI_ZOOM_KEY, ZOOM_STEP } from '../appearance';
import { runEditCommand, type EditCommand } from '../editCommands';
import { showErrorToast } from '../errors';
import { DOCUMENTATION_URL, ISSUES_URL } from '../links';
import { requestResync } from '../session';
import { dialogsStore, lockStore, numberSetting, settingsStore, updateView, viewStore } from '../store';

async function changeZoom(next: number): Promise<void> {
  const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, normalizeZoom(next)));
  const values = await settingsSet({ values: { [UI_ZOOM_KEY]: zoom } });
  settingsStore.set(values);
}

const currentZoom = () => normalizeZoom(numberSetting(settingsStore.get(), UI_ZOOM_KEY, 1));

function edit(command: EditCommand) {
  return () => runEditCommand(command);
}

/** Action ids (= app menu test ids) and what they do. */
export const APP_MENU_ACTIONS: Readonly<Record<string, () => void | Promise<unknown>>> = {
  'app-menu-vaults': () => activateTab('vaults'),
  'app-menu-settings': () => updateView({ settingsOpen: true }),
  'app-menu-new-local-terminal': () => openLocalTerminal(),
  'app-menu-edit-undo': edit('undo'),
  'app-menu-edit-redo': edit('redo'),
  'app-menu-edit-cut': edit('cut'),
  'app-menu-edit-copy': edit('copy'),
  'app-menu-edit-paste': edit('paste'),
  'app-menu-edit-select-all': edit('select-all'),
  'app-menu-view-zoom-in': () => changeZoom(currentZoom() + ZOOM_STEP),
  'app-menu-view-zoom-out': () => changeZoom(currentZoom() - ZOOM_STEP),
  'app-menu-view-zoom-reset': () => changeZoom(1),
  'app-menu-view-fullscreen': async () => {
    const next = !viewStore.get().fullscreen;
    await getBackend().window.setFullscreen(next);
    updateView({ fullscreen: next });
  },
  'app-menu-view-toggle-sidebar': () => updateView({ sidebarVisible: !viewStore.get().sidebarVisible }),
  'app-menu-window-new-window': () => windowOpen(),
  'app-menu-window-next-tab': () => activateNextTab(),
  'app-menu-window-previous-tab': () => activatePreviousTab(),
  'app-menu-window-close-tab': () => closeActiveTab(),
  'app-menu-help-documentation': () => externalOpenUrl(DOCUMENTATION_URL),
  'app-menu-help-report-issue': () => externalOpenUrl(ISSUES_URL),
  'app-menu-help-about': () => dialogsStore.set({ ...dialogsStore.get(), about: true }),
  'app-menu-lock': async () => {
    await lock();
    requestResync();
  },
  // Asks first while sessions are active (UI 1.4, 4.6).
  'app-menu-exit': () => requestAppExit(),
};

/**
 * Runs an app menu action. Only while unlocked: the menu is not shown
 * otherwise, and native menu clicks on a locked window are ignored.
 */
export function runAppMenuAction(id: string): void {
  const action = APP_MENU_ACTIONS[id];
  if (action === undefined || lockStore.get()?.state !== 'unlocked' || viewStore.get().firstRunInProgress) {
    return;
  }
  try {
    const result = action();
    if (result instanceof Promise) {
      result.catch(showErrorToast);
    }
  } catch (reason) {
    showErrorToast(reason);
  }
}
