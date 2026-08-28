// @vitest-environment jsdom

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AppProviders, resolveRuntimeMode } from '@/app/app-providers';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

vi.mock('virtual:pwa-register/react', () => ({
  useRegisterSW: () => ({
    needRefresh: [false, vi.fn()],
    updateServiceWorker: vi.fn(),
  }),
}));

const config = {
  controlPlaneUrl: 'https://api.agent-room.test',
  matrixHomeserverUrl: 'https://matrix.agent-room.test',
  registrationMode: 'open-email' as const,
  windowsDownloadUrl: null,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('应用组合根', () => {
  it('桌面构建优先使用设备运行时，即使原生检测暂时不可用', () => {
    expect(resolveRuntimeMode('desktop', false)).toBe('desktop');
    expect(resolveRuntimeMode('production', true)).toBe('desktop');
    expect(resolveRuntimeMode('production', false)).toBe('web');
  });

  it('桌面模式直接渲染产品连接舱且不创建 Web 网络会话', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch');
    window.history.replaceState(null, '', '/');

    render(
      <I18nextProvider i18n={i18n}>
        <AppProviders config={config} runtimeMode="desktop" />
      </I18nextProvider>,
    );

    expect(
      await screen.findByRole('heading', { name: 'Starting the local Agent runtime' }),
    ).not.toBeNull();
    expect(screen.queryByText('Let real agents enter the same room.')).toBeNull();
    expect(fetch).not.toHaveBeenCalled();
  });
});
