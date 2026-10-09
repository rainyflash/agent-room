// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { I18nextProvider } from 'react-i18next';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { DesktopAppCard } from './desktop-app-card';
import { RELEASES_URL, type PublishedDownload } from './use-published-download';

function renderCard(download: PublishedDownload) {
  return render(
    <I18nextProvider i18n={i18n}>
      <DesktopAppCard download={download} />
    </I18nextProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
});

describe('网页端“我的 Agent”里的桌面应用', () => {
  it('按访客的系统给安装包，GitHub 上的所有版本也给', () => {
    renderCard({ platform: 'macos', url: 'https://downloads.test/agent-room.dmg' });

    expect(screen.getByRole('heading', { name: 'Desktop app' })).toBeVisible();
    expect(screen.getByRole('link', { name: 'Download for Mac' })).toHaveAttribute(
      'href',
      'https://downloads.test/agent-room.dmg',
    );
    expect(screen.getByRole('link', { name: 'All versions on GitHub' })).toHaveAttribute(
      'href',
      RELEASES_URL,
    );
  });

  it('没有这个系统的安装包时按钮变灰，照样能去 GitHub 找', () => {
    renderCard({ platform: 'linux', url: null });

    expect(screen.getByRole('button', { name: 'No download for your system' })).toBeDisabled();
    expect(screen.queryByRole('link', { name: /Download for/u })).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'All versions on GitHub' })).toHaveAttribute(
      'href',
      RELEASES_URL,
    );
  });
});
