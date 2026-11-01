import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The UI lives in ui/. Tauri serves the built files from dist/ and the dev server on 1420.
export default defineConfig({
  root: 'ui',
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/target/**', '**/crates/**'] },
  },
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    target: 'es2022',
    chunkSizeWarningLimit: 12000,
  },
  worker: { format: 'es' },
  envPrefix: ['VITE_', 'TAURI_'],
});
