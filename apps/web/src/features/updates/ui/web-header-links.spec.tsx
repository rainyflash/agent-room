// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { useOptionalAppServices } from '@/app/app-services';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { REPOSITORY_URL } from './use-published-download';
import { WebHeaderLinks } from './web-header-links';

vi.mock('@/app/app-services', () => ({ useOptionalAppServices: vi.fn() }));
vi.mock('@/features/desktop/ui/desktop-runtime-provider', () => ({
  useOptionalDesktopRuntimeController: vi.fn(),
}));

const MAC_USER_AGENT =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';
const WINDOWS_USER_AGENT =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';
const LINUX_USER_AGENT =
  'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

function renderLinks({
  desktop = false,
  userAgent = MAC_USER_AGENT,
}: { readonly desktop?: boolean; readonly userAgent?: string } = {}) {
  vi.stubGlobal('navigator', { languages: ['en'], userAgent });
  // 读不到运行时清单时按页面配置给安装包。
  const fetch = vi.fn().mockRejectedValue(new TypeError('offline'));
  vi.stubGlobal('fetch', fetch);
  vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue(
    desktop
      ? ({ available: true } as unknown as ReturnType<typeof useOptionalDesktopRuntimeController>)
      : null,
  );
  vi.mocked(useOptionalAppServices).mockReturnValue({
    config: {
      windowsDownloadUrl: 'https://download.agent-room.test/installer.exe',
      macosDownloadUrl: 'https://download.agent-room.test/installer.dmg',
    },
  } as unknown as ReturnType<typeof useOptionalAppServices>);
  render(
    <I18nextProvider i18n={i18n}>
      <WebHeaderLinks />
    </I18nextProvider>,
  );
  return fetch;
}

describe('网页顶栏的 GitHub 和下载应用', () => {
  it('按访客的系统直接给安装包，提示里说是哪个系统的；GitHub 也在', () => {
    renderLinks();

    expect(screen.getByRole('link', { name: 'Source code on GitHub' })).toHaveAttribute(
      'href',
      REPOSITORY_URL,
    );
    const download = screen.getByRole('link', { name: 'Download app' });
    expect(download).toHaveAttribute('href', 'https://download.agent-room.test/installer.dmg');
    expect(download).toHaveAttribute('title', 'Download for Mac');
  });

  it('Windows 上给 Windows 的安装包', () => {
    renderLinks({ userAgent: WINDOWS_USER_AGENT });

    const download = screen.getByRole('link', { name: 'Download app' });
    expect(download).toHaveAttribute('href', 'https://download.agent-room.test/installer.exe');
    expect(download).toHaveAttribute('title', 'Download for Windows');
  });

  it('访客的系统没有安装包时只给 GitHub', () => {
    renderLinks({ userAgent: LINUX_USER_AGENT });

    expect(screen.getByRole('link', { name: 'Source code on GitHub' })).toBeVisible();
    expect(screen.queryByRole('link', { name: 'Download app' })).not.toBeInTheDocument();
  });

  it('桌面端里不显示，也不去读运行时清单', () => {
    const fetch = renderLinks({ desktop: true });

    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(fetch).not.toHaveBeenCalled();
  });
});
