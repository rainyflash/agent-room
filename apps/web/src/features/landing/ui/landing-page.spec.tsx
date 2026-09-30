// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import {
  LINUX_VISITOR,
  MACOS_VISITOR,
  WINDOWS_VISITOR,
  setVisitorSystem,
} from '@/test/visitor-system';
import { LandingPage } from '@/features/landing/ui/landing-page';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

vi.mock('@/app/app-services', () => ({ useAppServices: vi.fn() }));
vi.mock('motion/react', () => ({
  motion: { aside: 'aside', div: 'div' },
  useReducedMotion: () => true,
}));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  setVisitorSystem(WINDOWS_VISITOR);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe('公开 Alpha 首页', () => {
  it('只有配置版本化资产时才提供下载链接', () => {
    configure('https://download.agent-room.test/v0.1.0-alpha.1/installer.exe');

    renderPage();

    expect(screen.getByRole('link', { name: 'Download for Windows' })).toHaveAttribute(
      'href',
      'https://download.agent-room.test/v0.1.0-alpha.1/installer.exe',
    );
  });

  it('资产尚未发布时显示不可点击的可诊断状态', () => {
    configure(null);

    renderPage();

    expect(screen.getByRole('button', { name: 'No download for your system' })).toBeDisabled();
    expect(screen.queryByRole('link', { name: 'Download for Windows' })).not.toBeInTheDocument();
  });

  it('Mac 访客拿到磁盘映像，而不是 Windows 安装包', () => {
    setVisitorSystem(MACOS_VISITOR);
    configure(
      'https://download.agent-room.test/v0.1.0-alpha.1/installer.exe',
      'https://download.agent-room.test/v0.1.0-alpha.1/agent-room.dmg',
    );

    renderPage();

    expect(screen.getByRole('link', { name: 'Download for Mac' })).toHaveAttribute(
      'href',
      'https://download.agent-room.test/v0.1.0-alpha.1/agent-room.dmg',
    );
    expect(screen.getByText(/Apple silicon disk image/u)).toBeVisible();
  });

  it('还没有安装包的系统看到的是浏览器路径，不是别人的安装包', () => {
    setVisitorSystem(LINUX_VISITOR);
    configure('https://download.agent-room.test/v0.1.0-alpha.1/installer.exe');

    renderPage();

    expect(screen.getByRole('button', { name: 'No download for your system' })).toBeDisabled();
    expect(screen.getByText(/No desktop build for your system yet/u)).toBeVisible();
  });

  it('注册入口发送明确的注册意图', async () => {
    const user = userEvent.setup();
    const beginAuthentication = vi.fn();
    vi.mocked(useAppServices).mockReturnValue({
      config: { registrationMode: 'open-email', windowsDownloadUrl: null, macosDownloadUrl: null },
      controlPlane: { beginAuthentication, readSession: signedOut },
    } as unknown as ReturnType<typeof useAppServices>);

    renderPage();
    await user.click(await screen.findByRole('button', { name: 'Create account' }));

    expect(beginAuthentication).toHaveBeenCalledWith('/connect', 'register');
  });

  it('服务端关闭注册时不给用户死入口', async () => {
    configure(null);

    renderPage();

    expect(await screen.findByRole('button', { name: 'Registration coming soon' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Create account' })).not.toBeInTheDocument();
  });

  it('已登录时顶栏直接给“进入房间”，不再显示登录和注册', async () => {
    const readSession = vi.fn(() =>
      Promise.resolve(
        ok({
          authenticatedAtUnixMs: 1_800_000_000_000,
          displayName: 'Operator',
          expiresAtUnixMs: 1_900_000_000_000,
          locale: 'en',
          matrixUserId: '@operator:matrix.test',
          principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
          recentlyAuthenticated: false,
        }),
      ),
    );
    configure(null, null, readSession);

    renderPage();

    await waitFor(() => {
      expect(screen.getAllByRole('link', { name: 'Enter the room' })).toHaveLength(2);
    });
    for (const link of screen.getAllByRole('link', { name: 'Enter the room' })) {
      expect(link).toHaveAttribute('href', '/rooms');
    }
    expect(screen.queryByRole('button', { name: 'Log in' })).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Registration coming soon' }),
    ).not.toBeInTheDocument();
  });

  it('没登录时首页的“进入房间”去连接页，顶栏是登录和注册', async () => {
    configure(null);

    renderPage();

    expect(await screen.findByRole('button', { name: 'Log in' })).toBeEnabled();
    expect(screen.getByRole('link', { name: 'Enter the room' })).toHaveAttribute(
      'href',
      '/connect',
    );
  });
});

function signedOut() {
  return Promise.resolve(
    err({
      boundary: 'control-plane',
      code: 'authentication.session_required',
      offline: false,
      retryable: false,
    }),
  );
}

function configure(
  windowsDownloadUrl: string | null,
  macosDownloadUrl: string | null = null,
  readSession: () => Promise<unknown> = signedOut,
) {
  vi.mocked(useAppServices).mockReturnValue({
    config: { registrationMode: 'closed', windowsDownloadUrl, macosDownloadUrl },
    controlPlane: { beginAuthentication: vi.fn(), readSession },
  } as unknown as ReturnType<typeof useAppServices>);
}

function renderPage() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <RouterTestProvider>
          <LandingPage />
        </RouterTestProvider>
      </QueryClientProvider>
    </I18nextProvider>,
  );
}
