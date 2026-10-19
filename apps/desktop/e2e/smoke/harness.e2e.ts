// Harness self-check (not a UI section spec): the driver starts the e2e
// binary with a fresh data directory, the page loads, and a restart keeps the
// directory. Works against the stage-1 skeleton app (which has no app shell
// yet) and the full app. Run with:
//   pnpm --filter @shellpp/desktop e2e -- --spec e2e/smoke/harness.e2e.ts

import fs from 'node:fs';
import { exists } from '../helpers/index.ts';

async function documentReady(): Promise<void> {
  await browser.waitUntil(
    async () => (await browser.execute(() => document.readyState)) === 'complete',
    { timeout: 30_000, timeoutMsg: 'the page did not finish loading' },
  );
}

describe('e2e harness', () => {
  let dataDir: string;

  it('starts the app with a fresh temporary data directory', async () => {
    dataDir = process.env.SHELLPP_E2E_DATA_DIR ?? '';
    expect(dataDir).not.toBe('');
    expect(fs.existsSync(dataDir)).toBe(true);
    const handles = await browser.getWindowHandles();
    expect(handles.length).toBeGreaterThan(0);
    await documentReady();
  });

  it('loads the bundled e2e frontend', async () => {
    const origin = await browser.execute(() => window.location.origin);
    // The custom protocol of a built Tauri app, never the Vite dev server.
    expect(origin).not.toContain('localhost:5173');
    if (await exists('app-root')) {
      const helper = await browser.execute(() => typeof window.__SHELLPP_E2E__);
      expect(helper).toBe('object');
    }
  });

  it('restarts the app in the same data directory', async () => {
    const before = browser.sessionId;
    await browser.reloadSession();
    expect(browser.sessionId).not.toBe(before);
    expect(process.env.SHELLPP_E2E_DATA_DIR).toBe(dataDir);
    await documentReady();
    if (await exists('app-root')) {
      // The real app writes its database into SHELLPP_DATA_DIR.
      expect(fs.readdirSync(dataDir).length).toBeGreaterThan(0);
    }
  });
});
