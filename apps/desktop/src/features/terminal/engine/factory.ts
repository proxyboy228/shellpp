// Picks the terminal engine: xterm.js in the app and in `pnpm dev`, the fake
// engine in Vitest (no xterm.js in jsdom; stage 2 plan, section 9 S6).
// Both are loaded on demand, so xterm.js is a separate chunk of the bundle.
//
// Owner: S6.

import type { EngineCreateOptions, EngineFactory, TerminalEngine } from './types';

let override: EngineFactory | null = null;

/** Replaces the engine factory (tests); null restores the default. */
export function setEngineFactory(factory: EngineFactory | null): void {
  override = factory;
}

export async function createEngine(options: EngineCreateOptions): Promise<TerminalEngine> {
  if (override !== null) {
    return override(options);
  }
  if (import.meta.env.MODE === 'test') {
    const { createFakeEngine } = await import('./fakeEngine');
    return createFakeEngine(options);
  }
  const { createXtermEngine } = await import('./xtermEngine');
  return createXtermEngine(options);
}
