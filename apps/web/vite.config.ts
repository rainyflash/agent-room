import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';
import { VitePWA } from 'vite-plugin-pwa';

import { navigationFallbackDenylist } from './src/shared/pwa/navigation-fallback.js';

export default defineConfig(({ mode }) => ({
  plugins: [
    react(),
    tailwindcss(),
    VitePWA({
      disable: mode === 'capacity' || mode === 'desktop' || mode === 'vertical',
      injectRegister: false,
      manifest: {
        name: 'Agent Room',
        short_name: 'Agent Room',
        description: 'A shared room for people and their AI agents.',
        theme_color: '#247a77',
        background_color: '#ffffff',
        display: 'standalone',
        start_url: '/connect',
        icons: [
          {
            src: '/icons/192x192.png',
            sizes: '192x192',
            type: 'image/png',
            purpose: 'any maskable',
          },
          {
            src: '/icons/512x512.png',
            sizes: '512x512',
            type: 'image/png',
            purpose: 'any maskable',
          },
        ],
      },
      registerType: 'prompt',
      workbox: {
        cleanupOutdatedCaches: true,
        globPatterns: ['**/*.{html,js,css,svg}', 'studio/*.png', 'icons/*.png'],
        maximumFileSizeToCacheInBytes: 3 * 1024 * 1024,
        navigateFallback: '/index.html',
        navigateFallbackDenylist: [...navigationFallbackDenylist],
      },
    }),
  ],
  resolve: {
    alias: [{ find: '@', replacement: new URL('./src', import.meta.url).pathname }],
  },
  server: {
    allowedHosts: ['app.agent-room.localhost', 'localhost', '127.0.0.1'],
    host: '0.0.0.0',
    port: 5_173,
    strictPort: true,
  },
  build: {
    reportCompressedSize: true,
    ...(mode === 'capacity'
      ? {
          rollupOptions: {
            input: {
              accountWorkspaceFixture: 'e2e/fixtures/account-workspace.html',
              application: 'index.html',
              lobbyCapacityFixture: 'e2e/fixtures/lobby-scene.html',
              roomDirectoryFixture: 'e2e/fixtures/room-directory.html',
            },
          },
        }
      : {}),
    sourcemap: true,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    restoreMocks: true,
  },
}));
