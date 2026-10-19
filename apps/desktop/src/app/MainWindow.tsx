// The unlocked window content: the Vaults tab, the Settings window and the
// About dialog, with the app shortcuts and Edit menu tracking (activity
// reporting runs in AppRoot, so the first-run recovery key step counts too). Unmounted on lock, which drops every decrypted view model
// (plan, decision 19); the view state in `viewStore` survives.
//
// Owner: W4.

import { useEffect } from 'react';
import { SettingsModal } from '../features/settings';
import { AboutDialog } from './AboutDialog/AboutDialog';
import { installEditTracking } from './editCommands';
import { installShortcuts } from './shortcuts';
import { dialogsStore, updateView, useDialogs, useOs, useViewState } from './store';
import { VaultsTab } from './VaultsTab/VaultsTab';

export function MainWindow() {
  const os = useOs();
  const view = useViewState();
  const dialogs = useDialogs();

  useEffect(() => installShortcuts(os), [os]);
  useEffect(() => installEditTracking(), []);

  return (
    <>
      <VaultsTab />
      <SettingsModal open={view.settingsOpen} onClose={() => updateView({ settingsOpen: false })} />
      <AboutDialog open={dialogs.about} onClose={() => dialogsStore.set({ ...dialogsStore.get(), about: false })} />
    </>
  );
}

export default MainWindow;
