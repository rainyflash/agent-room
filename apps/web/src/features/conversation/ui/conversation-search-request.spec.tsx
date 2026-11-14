// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

import { ConversationPanel } from './conversation-panel';
import { ConversationSearchRequestContext } from './conversation-search-request';

const ROOM = '!chat:agent-room.test';
const said: readonly RoomMessageSignal[] = [
  conversationFixture('01990d9e-8400-7000-8000-000000004001', { roomId: ROOM }),
];

const publisher: MessagePublisher = {
  publish: () => Promise.reject(new Error('这些用例不发消息。')),
  reconcile: () => Promise.reject(new Error('这些用例不发消息。')),
  resolveIdentity: () =>
    Promise.resolve(
      ok({
        kind: 'human',
        displayName: 'Rainy',
        matrixUserId: '@rainy:agent-room.test',
        principalId: '01990d9e-8400-7000-8000-000000000009',
        source: 'matrix_human_session',
      }),
    ),
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
  // jsdom 不排版，没有这个方法。
  Element.prototype.scrollIntoView = vi.fn();
});
afterEach(cleanup);

/** 房间页那一头：`turn` 是请求了几次，`onAvailable` 收对话报的“有没有可查的”。 */
async function renderPanel(options: {
  readonly active?: boolean;
  readonly messages?: readonly RoomMessageSignal[];
  readonly onAvailable?: (available: boolean) => void;
}) {
  const onAvailable = options.onAvailable ?? (() => undefined);
  const panel = (turn: number): ReactNode => (
    <I18nextProvider i18n={i18n}>
      <ConversationSearchRequestContext.Provider value={{ turn, onAvailable }}>
        <ConversationPanel
          active={options.active ?? true}
          messages={options.messages ?? said}
          publisher={publisher}
          roomId={ROOM}
          state="ready"
        />
      </ConversationSearchRequestContext.Provider>
    </I18nextProvider>
  );
  const view = render(panel(0));
  await waitFor(() => {
    expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
  });
  return {
    request: (turn: number) => {
      view.rerender(panel(turn));
    },
  };
}

const search = () => screen.queryByRole('search', { name: 'Search this conversation' });

describe('从对话外面打开查找', () => {
  it('请求一次：查找打开，焦点落在关键词那一格；对话先报了有可查的', async () => {
    const onAvailable = vi.fn();
    const view = await renderPanel({ onAvailable });
    expect(onAvailable).toHaveBeenLastCalledWith(true);
    expect(search()).toBeNull();

    view.request(1);
    expect(search()).toBeVisible();
    await waitFor(() => {
      expect(screen.getByRole('searchbox')).toHaveFocus();
    });
    // 顶上那一行的按钮照旧能把它关上。
    expect(screen.getByRole('button', { name: 'Search this conversation' })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
  });

  it('不在眼前的那一段对话不应，也不报', async () => {
    const onAvailable = vi.fn();
    const view = await renderPanel({ active: false, onAvailable });
    view.request(1);
    expect(search()).toBeNull();
    expect(onAvailable).not.toHaveBeenCalled();
  });

  it('一条消息都没有：报没有可查的，请求了也不开', async () => {
    const onAvailable = vi.fn();
    const view = await renderPanel({ messages: [], onAvailable });
    expect(onAvailable).toHaveBeenLastCalledWith(false);
    view.request(1);
    expect(search()).toBeNull();
    expect(screen.queryByRole('button', { name: 'Search this conversation' })).toBeNull();
  });
});
