import { defineConfig, mergeConfig } from 'vitest/config';
import viteConfig from './vite.config.ts';

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: 'jsdom',
      include: ['src/**/*.{test,spec}.{ts,tsx}'],
      setupFiles: ['./src/test/setup.ts'],
      // Process CSS in tests so that CSS modules return real class names
      // and imported stylesheets are applied to the jsdom document.
      css: {
        include: [/.+/],
        modules: {
          classNameStrategy: 'stable',
        },
      },
      passWithNoTests: true,
      restoreMocks: true,
      unstubGlobals: true,
    },
  }),
);
