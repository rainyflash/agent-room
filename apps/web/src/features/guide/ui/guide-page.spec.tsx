// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import { GuidePage } from '@/features/guide/ui/guide-page';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

vi.mock('@/app/app-services', () => ({ useAppServices: vi.fn() }));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe('使用指南', () => {
  it('三步走完就能聊，桌面应用是后面可选的一节，并给出源码与自托管入口', () => {
    const { container } = renderGuide('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');

    const list = container.querySelector('.guide__steps');
    if (!(list instanceof HTMLElement)) throw new Error('Missing guide steps');
    const steps = within(list);
    expect(steps.getAllByRole('listitem')).toHaveLength(3);
    for (const title of [
      'Create an account',
      'Bring an agent in',
      'Talk, and let it answer while you are away',
    ]) {
      expect(steps.getByRole('heading', { name: title })).toBeVisible();
    }
    const desktop = screen.getByRole('region', { name: 'The desktop app' });
    expect(within(desktop).getByText('Optional')).toBeVisible();
    expect(within(desktop).getByText(/Only needed to bring in an agent/u)).toBeVisible();
    expect(screen.queryByText(/Bridge/u)).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Source code on GitHub' })).toHaveAttribute(
      'href',
      'https://github.com/rainyflash/agent-room',
    );
    expect(screen.getByRole('link', { name: 'Run your own server' })).toBeVisible();
  });

  it('Mac 访客拿到已公证的磁盘映像，不再被教着绕过 Gatekeeper', () => {
    renderGuide('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', {
      windowsDownloadUrl: 'https://download.agent-room.test/installer.exe',
      macosDownloadUrl: 'https://download.agent-room.test/agent-room.dmg',
    });

    expect(screen.getByText(/You are on a Mac/u)).toBeVisible();
    expect(screen.getByText(/notarized by Apple/u)).toBeVisible();
    expect(screen.queryByText(/Privacy & Security/u)).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Download the desktop app' })).toHaveAttribute(
      'href',
      'https://download.agent-room.test/agent-room.dmg',
    );
  });

  it('装不了桌面端的访客先看到自己的系统没有安装包', () => {
    renderGuide('Mozilla/5.0 (X11; Linux x86_64)', {
      windowsDownloadUrl: 'https://download.agent-room.test/installer.exe',
      macosDownloadUrl: 'https://download.agent-room.test/agent-room.dmg',
    });

    expect(screen.getByText(/There is no desktop build for your system yet/u)).toBeVisible();
    expect(
      screen.queryByRole('link', { name: 'Download the desktop app' }),
    ).not.toBeInTheDocument();
  });

  it('答复了「别人能不能指挥我的 Agent」', () => {
    renderGuide('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');

    expect(screen.getByText(/only speaks on its own after you allow it/u)).toBeVisible();
  });
});

function renderGuide(
  userAgent: string,
  config: {
    readonly windowsDownloadUrl: string | null;
    readonly macosDownloadUrl: string | null;
  } = {
    windowsDownloadUrl: 'https://download.agent-room.test/installer.exe',
    macosDownloadUrl: null,
  },
) {
  vi.stubGlobal('navigator', { languages: ['en'], userAgent });
  vi.mocked(useAppServices).mockReturnValue({ config } as unknown as ReturnType<
    typeof useAppServices
  >);
  return render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <GuidePage />
      </RouterTestProvider>
    </I18nextProvider>,
  );
}
