import { defineConfig } from 'vitest/config';

export default defineConfig({
  resolve: {
    alias: {
      '@': new URL('./apps/web/src', import.meta.url).pathname,
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
