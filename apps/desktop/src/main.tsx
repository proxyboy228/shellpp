// Entry point of every app window (plan, "Frontend: module map").
//
// - Selects the IPC backend (Tauri in the app; the in-browser mock in
//   development without Tauri).
// - e2e builds (`vite build --mode e2e`) expose `window.__SHELLPP_E2E__`.
// - Development only: `#/showcase` renders the stage 0 design system page;
//   production and e2e builds do not contain it.
// - Freezes `Object.prototype` before the backend starts (docs/security.md,
//   section 13.1). Tauri's `freezePrototype` would freeze it before any
//   script runs, but xterm.js 6 assigns `toString` to a plain object while
//   its module initializes (a namespace of its bundled keyboard code), which
//   throws once `Object.prototype` is frozen. So the terminal engine is
//   loaded first, then the prototype is frozen, before any IPC data or
//   terminal output is processed.

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './theme/tokens.css';
import { AppRoot } from './app/AppRoot';
import { initBackend } from './ipc';
import { installE2eHelper } from './ipc/e2e';

const rootElement = document.getElementById('root');
if (rootElement === null) {
  throw new Error('Missing #root element in index.html');
}
const root = createRoot(rootElement);

/** Loads the terminal engine, then freezes `Object.prototype` (see above). */
async function freezePrototype(): Promise<void> {
  try {
    await import('./features/terminal/engine/xtermEngine');
  } catch {
    // The terminal views try again when they open; the freeze applies anyway.
  }
  Object.freeze(Object.prototype);
}

async function start(): Promise<void> {
  await freezePrototype();
  if (import.meta.env.DEV && window.location.hash === '#/showcase') {
    const { default: Showcase } = await import('./showcase/App');
    root.render(
      <StrictMode>
        <Showcase />
      </StrictMode>,
    );
    return;
  }
  await initBackend();
  if (import.meta.env.MODE === 'e2e') {
    installE2eHelper();
  }
  root.render(
    <StrictMode>
      <AppRoot />
    </StrictMode>,
  );
}

start().catch((error: unknown) => {
  // Outside the desktop app a production build has no backend: show why.
  rootElement.textContent = error instanceof Error ? error.message : String(error);
});
