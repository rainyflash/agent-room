// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { privacyResources } from '@/features/privacy/i18n/privacy-resources';
import { PrivacyPage } from '@/features/privacy/ui/privacy-page';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
});

describe('隐私说明', () => {
  it('分节说清存了什么、谁能读到、保留多久、怎么删，并给出联系方式', () => {
    renderPage();

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Privacy');
    expect(
      screen.getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent),
    ).toEqual([
      'In short',
      'What we keep',
      'Who can read what',
      'Who else handles it',
      'How long we keep it',
      'Download or delete your data',
      'Questions and requests',
      'Changes',
      'More',
    ]);
    expect(screen.getByRole('link', { name: 'Open an issue' })).toHaveAttribute(
      'href',
      'https://github.com/rainyflash/agent-room/issues',
    );
    expect(screen.getByRole('link', { name: 'Security policy' })).toHaveAttribute(
      'href',
      'https://github.com/rainyflash/agent-room/blob/main/SECURITY.md',
    );
  });

  it('照实说运营方读得到私人房间，不把加密说成服务器读不到', () => {
    renderPage();

    expect(
      screen.getByText(/Whoever runs the server can therefore read private rooms/u),
    ).toBeVisible();
    expect(screen.queryByText(/can(?:no|’)t read/u)).not.toBeInTheDocument();
  });

  it('每条小标题都有正文，中英文条目一一对应，页面上不出现邮箱地址', () => {
    const keys = Object.keys(privacyResources.en);
    expect(Object.keys(privacyResources['zh-CN'])).toEqual(keys);
    for (const label of keys.filter((key) => key.endsWith('.label'))) {
      expect(keys).toContain(label.replace(/\.label$/u, '.text'));
    }
    for (const text of [
      ...Object.values(privacyResources.en),
      ...Object.values(privacyResources['zh-CN']),
    ]) {
      expect(text).not.toMatch(/[\w.+-]+@[\w-]+\.\w+/u);
    }
  });
});

function renderPage() {
  return render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <PrivacyPage />
      </RouterTestProvider>
    </I18nextProvider>,
  );
}
