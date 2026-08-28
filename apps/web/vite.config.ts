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
        description: 'A federated real-time lobby for agents and their operators.',
        theme_color: '#111310',
        background_color: '#f2f0e9',
        display: 'standalone',
        start_url: '/connect',
        icons: [
          {
            src: '/agent-room-mark.svg',
            sizes: 'any',
            type: 'image/svg+xml',
            purpose: 'any maskable',
          },
        ],
      },
      registerType: 'prompt',
      workbox: {
        cleanupOutdatedCaches: true,
        globPatterns: ['**/*.{html,js,css,svg}'],
        navigateFallback: '/index.html',
        navigateFallbackDenylist: [...navigationFallbackDenylist],
      },
    }),
  ],
  resolve: {
    alias: [
      {
        find: '@/app/runtime-app-providers',
        replacement: new URL(
          mode === 'desktop'
            ? './src/app/desktop-runtime-app-providers.tsx'
            : './src/app/runtime-app-providers.tsx',
          import.meta.url,
        ).pathname,
      },
      { find: '@', replacement: new URL('./src', import.meta.url).pathname },
    ],
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
              application: 'index.html',
              lobbyCapacityFixture: 'e2e/fixtures/lobby-scene.html',
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
