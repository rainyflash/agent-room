// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { GuidePage } from '@/features/guide/ui/guide-page';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('使用指南', () => {
  it('四步都在，并给出源码与自托管入口', () => {
    renderGuide('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');

    for (const title of [
      'Create an account',
      'Install the Windows app',
      'Bring an agent in',
      'Talk, and let it reply while you are away',
    ]) {
      expect(screen.getByRole('heading', { name: title })).toBeVisible();
    }
    expect(screen.getByRole('link', { name: 'Source code on GitHub' })).toHaveAttribute(
      'href',
      'https://github.com/rainyflash/agent-room',
    );
    expect(screen.getByRole('link', { name: 'Run your own server' })).toBeVisible();
  });

  it('装不了桌面端的访客先看到自己的系统没有安装包', () => {
    renderGuide('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)');

    expect(screen.getByText(/You are on a Mac\./u)).toBeVisible();
    expect(screen.queryByText(/You are on Windows/u)).not.toBeInTheDocument();
  });

  it('答复了「别人能不能指挥我的 Agent」', () => {
    renderGuide('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');

    expect(screen.getByText(/Replying on its own requires a grant you create/u)).toBeVisible();
  });
});

function renderGuide(userAgent: string) {
  vi.stubGlobal('navigator', { languages: ['en'], userAgent });
  return render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <GuidePage />
      </RouterTestProvider>
    </I18nextProvider>,
  );
}
