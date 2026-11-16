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

  it('照实说算提醒时替你读什么、不读什么（specs/redesign/reminders.md 的 4R-4c）', () => {
    renderPage();

    expect(screen.getByText(/works out who has new messages/u)).toBeVisible();
    expect(privacyResources.en['privacy.keep.reminders.text']).toMatch(/not at what it says/u);
    expect(privacyResources.en['privacy.keep.reminders.text']).toMatch(/never who/u);
    expect(privacyResources['zh-CN']['privacy.keep.reminders.text']).toMatch(/不看说了什么/u);
    expect(privacyResources['zh-CN']['privacy.keep.reminders.text']).toMatch(/不记是谁/u);
  });

  it('照实说绑仓库时读 GitHub 上哪两个文件、不存内容（specs/redesign/project-rooms.md 的 3P-2b）', () => {
    renderPage();

    expect(screen.getByText('GitHub repositories', { selector: 'dt' })).toBeVisible();
    expect(screen.getByText(/reads those two files/u)).toBeVisible();
    expect(privacyResources.en['privacy.readers.repositories.text']).toMatch(
      /doesn’t store what’s in them/u,
    );
    expect(privacyResources.en['privacy.readers.repositories.text']).toMatch(/within 7 days/u);
    expect(privacyResources['zh-CN']['privacy.readers.repositories.text']).toMatch(/不存文件内容/u);
    expect(privacyResources['zh-CN']['privacy.readers.repositories.text']).toMatch(
      /7 天没核对上就删掉/u,
    );
    expect(privacyResources.en['privacy.others.github.text']).toMatch(/reads two files/u);
    expect(privacyResources['zh-CN']['privacy.others.github.text']).toMatch(
      /读那个仓库里的两个文件/u,
    );
  });

  it('照实说按粗地区汇总使用的总数，不再说不收集使用统计', () => {
    renderPage();

    expect(screen.getByText(/Totals by coarse region/u)).toBeVisible();
    expect(screen.getByText(/DB-IP’s free country database/u)).toBeVisible();
    // 周报里按注册的那一周数新用户走到哪一步（specs/onboarding-funnel/design.md）。
    expect(screen.getByText(/how many of the people who signed up each week/u)).toBeVisible();
    expect(privacyResources['zh-CN']['privacy.keep.usage.text']).toMatch(/每一周注册的人里/u);
    expect(privacyResources.en['privacy.keep.usage.text']).toMatch(/no account, room or region/u);
    expect(privacyResources['zh-CN']['privacy.keep.usage.text']).toMatch(/不带账户、房间和地区/u);
    for (const text of [
      ...Object.values(privacyResources.en),
      ...Object.values(privacyResources['zh-CN']),
    ]) {
      expect(text).not.toMatch(/no usage statistics|不收集使用统计|不做统计分析/u);
    }
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
