// The unlocked window content: the Vaults tab (kept mounted while a terminal
// tab is active, hidden), the Settings window, the About dialog and the Ctrl+K
// search palette, with the app and terminal shortcuts and Edit menu tracking
// (activity reporting runs in AppRoot, so the first-run recovery key step
// counts too). Unmounted on lock, which drops every decrypted view model
// (stage 1 plan, decision 19); the view state in `viewStore` survives. The
// terminals live in features/terminal `TerminalHost`, mounted by AppRoot, so
// they survive a lock.
//
// Owner: lead.

import { useEffect } from 'react';
import { SearchPalette } from '../features/newtab';
import { SettingsModal } from '../features/settings';
import { installTerminalShortcuts, useWorkspace } from '../features/terminal';
import { AboutDialog } from './AboutDialog/AboutDialog';
import { installEditTracking } from './editCommands';
import styles from './MainWindow.module.css';
import { installShortcuts } from './shortcuts';
import { dialogsStore, updateView, useDialogs, useOs, useViewState } from './store';
import { VaultsTab } from './VaultsTab/VaultsTab';

export function MainWindow() {
  const os = useOs();
  const view = useViewState();
  const dialogs = useDialogs();
  const workspace = useWorkspace();
  const vaultsActive = workspace.active === 'vaults';

  useEffect(() => installShortcuts(os), [os]);
  useEffect(() => installTerminalShortcuts(os), [os]);
  useEffect(() => installEditTracking(), []);

  return (
    <>
      <div className={styles.vaults} hidden={!vaultsActive}>
        <VaultsTab active={vaultsActive} />
      </div>
      <SearchPalette />
      <SettingsModal open={view.settingsOpen} onClose={() => updateView({ settingsOpen: false })} />
      <AboutDialog open={dialogs.about} onClose={() => dialogsStore.set({ ...dialogsStore.get(), about: false })} />
    </>
  );
}

export default MainWindow;
