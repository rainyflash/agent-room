import { defaultExclude, defineConfig } from 'vitest/config';
import { writeContract, applicationVersion } from './apps/web/build/runtime-manifest.js';

export default defineConfig({
  define: {
    __AGENT_ROOM_WRITE_CONTRACT__: JSON.stringify(writeContract),
    __AGENT_ROOM_VERSION__: JSON.stringify(applicationVersion),
  },
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
    // 本机工具目录可能包含完整检出的 git worktree。它不安装依赖，被收集进用例会让
    // 整套本地测试失败；CI 的干净检出没有该目录，因此这类失败只在本机出现。
    exclude: [...defaultExclude, '.claude/**'],
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
