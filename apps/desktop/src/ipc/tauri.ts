// The Tauri backend: `invoke` with a single `input` argument, window-scoped
// event listeners and the title bar window controls.
//
// Owner: W4.

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { Backend, WindowControls } from './backend';
import { toIpcError } from './errors';
import type { CommandInput, CommandName, CommandOutput, EventMap, EventName } from './types';

export function createTauriBackend(): Backend {
  const webviewWindow = getCurrentWebviewWindow();

  const windowControls: WindowControls = {
    label: webviewWindow.label,
    minimize: () => webviewWindow.minimize(),
    toggleMaximize: () => webviewWindow.toggleMaximize(),
    isMaximized: () => webviewWindow.isMaximized(),
    close: () => webviewWindow.close(),
    startDragging: () => webviewWindow.startDragging(),
    setFullscreen: (fullscreen) => webviewWindow.setFullscreen(fullscreen),
    onResized: (handler) => webviewWindow.onResized(() => handler()),
  };

  return {
    kind: 'tauri',
    async invoke<C extends CommandName>(command: C, input: CommandInput<C>): Promise<CommandOutput<C>> {
      try {
        // Every command takes at most one argument, named `input`.
        const args = input === undefined ? {} : { input };
        return await invoke<CommandOutput<C>>(command, args);
      } catch (reason) {
        throw toIpcError(reason);
      }
    },
    listen<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void) {
      // The window-scoped listener receives broadcast events and the events
      // targeted at this window (`link:open`, `menu:action`), not those of other windows.
      return webviewWindow.listen<EventMap[E]>(event, (received) => handler(received.payload));
    },
    window: windowControls,
  };
}
