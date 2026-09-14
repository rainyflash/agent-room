import { defineConfig } from 'vitest/config';
import { writeContract } from './apps/web/build/runtime-manifest.js';

export default defineConfig({
  define: { __AGENT_ROOM_WRITE_CONTRACT__: JSON.stringify(writeContract) },
  resolve: {
    alias: {
      '@': new URL('./apps/web/src', import.meta.url).pathname,
      'virtual:pwa-register/react': new URL(
        './apps/web/src/test/pwa-register-react-stub.ts',
        import.meta.url,
      ).pathname,
    },
  },
  test: {
    coverage: {
      include: ['packages/protocol/src/validator.ts'],
      provider: 'v8',
      reporter: ['text', 'json-summary'],
      thresholds: {
        branches: 90,
        functions: 100,
        lines: 100,
        statements: 100,
      },
    },
  },
});
