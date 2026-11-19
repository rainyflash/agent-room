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

  it('不做邮件提醒了，页面上不再有“提醒”那一条（specs/redesign/reminders.md）', () => {
    renderPage();

    expect(screen.queryByText(/email reminders/iu)).not.toBeInTheDocument();
    expect(Object.keys(privacyResources.en).filter((key) => key.includes('reminders'))).toEqual([]);
    expect(
      Object.keys(privacyResources['zh-CN']).filter((key) => key.includes('reminders')),
    ).toEqual([]);
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

  it('照实说分享页上的实况放什么、什么时候自己关掉（specs/redesign/project-rooms.md 的 3P-6a）', () => {
    renderPage();

    expect(screen.getByText(/turns on the live view/u)).toBeVisible();
    const english = privacyResources.en['privacy.readers.publicRooms.text'];
    expect(english).toMatch(/last 20 messages said in the main room after that moment/u);
    expect(english).toMatch(/sub-rooms are never shown/u);
    expect(english).toMatch(/goes back to private or is closed/u);
    const chinese = privacyResources['zh-CN']['privacy.readers.publicRooms.text'];
    expect(chinese).toMatch(/从那一刻起主房间里最近说的 20 句/u);
    expect(chinese).toMatch(/子房间一概不放/u);
    expect(chinese).toMatch(/改回私密、关掉时自动关掉/u);
    expect(privacyResources.en['privacy.delete.how']).toMatch(/live views you turned on/u);
    expect(privacyResources['zh-CN']['privacy.delete.how']).toMatch(/你打开的分享页实况/u);
  });

  it('照实说分享的一段对话存什么、不存话、什么时候删（specs/redesign/project-rooms.md 的 3P-7a）', () => {
    renderPage();

    expect(screen.getByText(/share a stretch of up to 30 messages/u)).toBeVisible();
    const english = privacyResources.en['privacy.readers.publicRooms.text'];
    expect(english).toMatch(/never the text/u);
    expect(english).toMatch(/doesn’t say who shared it/u);
    expect(english).toMatch(/older than the room’s retention period/u);
    // 打开时读到的在控制面内存里留 60 秒（3P-7a-2 的 `public_watch/excerpt_pages.rs`）。
    expect(english).toMatch(/in memory for a minute/u);
    const chinese = privacyResources['zh-CN']['privacy.readers.publicRooms.text'];
    expect(chinese).toMatch(/连着的最多 30 句分享成一个单独的链接/u);
    expect(chinese).toMatch(/不存话/u);
    expect(chinese).toMatch(/读到的在内存里留一分钟/u);
    expect(chinese).toMatch(/页面上不写是谁分享的/u);
    expect(privacyResources.en['privacy.delete.how']).toMatch(/conversation excerpts you shared/u);
    expect(privacyResources['zh-CN']['privacy.delete.how']).toMatch(/你分享的一段段对话/u);
    expect(privacyResources.en['privacy.delete.removed']).toMatch(
      /conversation excerpts you shared/u,
    );
    expect(privacyResources['zh-CN']['privacy.delete.removed']).toMatch(/你分享的一段段对话/u);
  });

  it('照实说分享链接的预览图只画本来公开的、不画说的话（specs/redesign/project-rooms.md 的 3P-8a）', () => {
    renderPage();

    expect(screen.getByText(/comes with a preview image that our server draws/u)).toBeVisible();
    const english = privacyResources.en['privacy.readers.publicRooms.text'];
    expect(english).toMatch(/never what’s said/u);
    expect(english).toMatch(/in memory for 10 minutes/u);
    const chinese = privacyResources['zh-CN']['privacy.readers.publicRooms.text'];
    expect(chinese).toMatch(/带一张我们的服务器画的预览图/u);
    expect(chinese).toMatch(/不画说的话/u);
    expect(chinese).toMatch(/画好的在服务器内存里留 10 分钟/u);
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
