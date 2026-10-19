// Ctrl+K / Ctrl+Shift+K search of hosts and open tabs from anywhere in the
// window (UI 6, 12.5; stage 2 plan, decision 13): a modal overlay with the
// host picker (`search-palette`). Choosing a tab activates it; choosing a
// host connects in a new tab (Alt: new window). Mounted once by MainWindow;
// it closes when MainWindow unmounts (lock).
//
// Owner: S7 (skeleton by the lead).

import { useEffect } from 'react';
import { showErrorToast } from '../../app/errors';
import { createStore, useStore } from '../../app/store/createStore';
import { Modal } from '../../components';
import type { SessionTarget } from '../../ipc';
import { t } from '../../i18n';
import { activateTab, openSession } from '../connect/workspaceBridge';
import { HostPicker } from './HostPicker';

const paletteStore = createStore<boolean>(false);

/** Opens the palette (app shortcuts, src/app/shortcuts.ts). */
export function openSearchPalette(): void {
  paletteStore.set(true);
}

/** Closes the palette. */
export function closeSearchPalette(): void {
  paletteStore.set(false);
}

/** True while the palette is open. */
export function isSearchPaletteOpen(): boolean {
  return paletteStore.get();
}

export function SearchPalette() {
  const open = useStore(paletteStore);

  // A lock unmounts the window content: the palette does not come back after unlock.
  useEffect(() => () => closeSearchPalette(), []);

  const pickHost = (target: SessionTarget, { altKey }: { altKey: boolean }) => {
    closeSearchPalette();
    openSession(target, altKey ? { newWindow: true } : {}).catch(showErrorToast);
  };

  const pickTab = (tabId: string) => {
    closeSearchPalette();
    activateTab(tabId);
  };

  return (
    <Modal
      open={open}
      size="medium"
      title={t('search_palette.title')}
      closeLabel={t('common.close')}
      onClose={closeSearchPalette}
      closeOnScrimClick
      data-testid="search-palette"
    >
      <HostPicker testIdPrefix="search-palette" includeTabs onPickHost={pickHost} onPickTab={pickTab} onCancel={closeSearchPalette} />
    </Modal>
  );
}
