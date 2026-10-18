import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Keep the Tauri CLI output visible once the desktop shell is added (stage 1).
  clearScreen: false,
  server: {
    // A fixed port: the Tauri dev configuration (stage 1) points at it,
    // so fail instead of silently switching to another port.
    port: 1420,
    strictPort: true,
  },
});
