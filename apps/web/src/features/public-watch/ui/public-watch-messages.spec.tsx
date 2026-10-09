// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import type { PublicWatch, PublicWatchMessage } from '@/features/public-watch/domain/public-watch';
import { PublicWatchMessages } from '@/features/public-watch/ui/public-watch-messages';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

const sentAt = Date.UTC(2026, 9, 20, 8, 0, 0);

function message(key: string, author: string, extra: Partial<PublicWatchMessage>) {
  return {
    key,
    author,
    text: '',
    truncated: false,
    withheld: false,
    attachment: false,
    edited: false,
    replyTo: null,
    sentAtUnixMs: sentAt,
    ...extra,
  } satisfies PublicWatchMessage;
}

const watch: PublicWatch = {
  schemaVersion: 1,
  lobby: {
    catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
    name: 'Agent Room Global',
    slug: 'agent-room-global',
  },
  participants: [
    { key: 'pSol', name: 'Sol', kind: 'networkAgent', online: true, status: 'idle' },
    { key: 'pAda', name: 'Ada', kind: 'agent', online: false, status: null },
    { key: 'pLin', name: 'Lin', kind: 'person', online: false, status: null },
  ],
  messages: [
    message('mHello', 'pSol', { text: 'Hello **everyone**' }),
    message('mSecret', 'pAda', { withheld: true }),
    message('mPhoto', 'pLin', { attachment: true, text: 'Look at this', edited: true }),
    message('mLong', 'pSol', { text: 'Long story', truncated: true, replyTo: 'mHello' }),
    message('mAnswer', 'pLin', { text: 'Agreed', replyTo: 'mSecret' }),
    message('mOld', 'pAda', { text: 'Following up', replyTo: 'mGoneFromPage' }),
  ],
  updatedAtUnixMs: sentAt + 3_000,
};

function renderMessages(value: PublicWatch = watch) {
  render(
    <I18nextProvider i18n={i18n}>
      <PublicWatchMessages focus={null} watch={value} />
    </I18nextProvider>,
  );
  return screen.getByRole('log', { name: 'Chat' });
}

describe('围观页的对话', () => {
  it('只能看：名字和身份、不公开的、带文件的、太长的、改过的、回复的都照实说', () => {
    const log = renderMessages();
    const articles = within(log).getAllByRole('article');
    const article = (index: number): HTMLElement => {
      const found = articles[index];
      if (found === undefined) throw new Error(`第 ${String(index)} 条消息不在`);
      return found;
    };

    expect(articles).toHaveLength(6);
    expect(within(article(0)).getByText('Network agent')).toBeVisible();
    expect(within(article(0)).getByText('everyone').tagName).toBe('STRONG');
    expect(within(article(1)).getByText('A message that isn’t shown publicly')).toBeVisible();
    expect(within(article(2)).getByText('Shared a file')).toBeVisible();
    expect(within(article(2)).getByText('Edited')).toBeVisible();
    expect(within(article(2)).getByText('Person')).toBeVisible();
    expect(within(article(3)).getByText('Only the beginning is shown.')).toBeVisible();
    expect(within(article(3)).getByText('Sol: Hello **everyone**')).toBeVisible();
    expect(within(article(4)).getByText('Ada: A message that isn’t shown publicly')).toBeVisible();
    expect(within(article(5)).getByText('Earlier message')).toBeVisible();
    // 只能看：没有输入框、回复和复制按钮，也不链到任何地方。
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
  });

  it('还没人说话时说清楚', () => {
    const log = renderMessages({ ...watch, messages: [] });

    expect(
      within(log).getByRole('heading', { name: 'No one has said anything yet' }),
    ).toBeVisible();
  });
});
