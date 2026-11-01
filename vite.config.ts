import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';

const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8'));
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed dev port; the dev server must be reachable by the webview.
const host = process.env.TAURI_DEV_HOST || '0.0.0.0';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  server: {
    host,
    port: 1420,
    strictPort: true,
    allowedHosts: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  preview: { host: '0.0.0.0', port: 4173, allowedHosts: true },
  build: {
    target: 'chrome105', // WebView2 (Chromium) on supported Windows builds
    sourcemap: false,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
