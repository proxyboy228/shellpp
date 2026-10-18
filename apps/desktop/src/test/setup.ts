// Test setup for Vitest (see `vitest.config.ts`).
import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// Vitest globals are disabled, so Testing Library cannot register its
// automatic cleanup. Unmount rendered trees after every test instead.
afterEach(() => {
  cleanup();
});

// jsdom does not implement `window.matchMedia`. Provide an inert default
// (no query matches) so that code reading `prefers-color-scheme` or
// `prefers-reduced-motion` can run. Tests that need a specific result
// replace it (the property is writable and configurable).
if (typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: (query: string): MediaQueryList => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => false,
    }),
  });
}
