// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { InboxNotice } from './inbox-notice';
import type { InboxItem } from '../domain/inbox';
import { systemNotificationSummary } from '../domain/inbox';
import {
  emptyWorkspace,
  workspaceIndex,
} from '@/features/personal-workspace/domain/workspace-document';
import { initializeI18n, i18n } from '@/shared/i18n/i18n';

const inbox = vi.hoisted(() => ({
  current: {
    accountId: 'account-1',
    items: [] as readonly InboxItem[],
    limited: false,
    unavailableRooms: 0,
    status: 'ready' as const,
  },
}));
const desktop = vi.hoisted(() => ({
  notify: vi.fn(() => Promise.resolve()),
  available: true,
}));

vi.mock('./inbox-provider', () => ({
  useInbox: () => ({ snapshot: inbox.current, refresh: () => undefined }),
}));
vi.mock('@/features/personal-workspace/ui/personal-workspace-provider', () => ({
  usePersonalWorkspace: () => ({
    snapshot: {
      accountId: 'account-1',
      document: emptyWorkspace,
      // 免打扰在过去到期 = 允许提醒。
      index: { ...workspaceIndex(emptyWorkspace), doNotDisturbUntil: 1 },
      status: 'synced',
      failure: null,
    },
  }),
}));
vi.mock('@/features/desktop/ui/desktop-runtime-provider', () => ({
  useOptionalDesktopRuntimeController: () =>
    desktop.available ? { available: true, notify: desktop.notify } : null,
}));
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children }: { readonly children: React.ReactNode }) => <a href="/inbox">{children}</a>,
  useLocation: () => '/lobby',
}));

function item(id: string, overrides: Partial<InboxItem> = {}): InboxItem {
  return {
    id,
    kind: 'mention',
    room: { catalogId: 'catalog', roomId: '!room:test', name: 'game dev', direct: false },
    messageId: id,
    conversation: true,
    sender: 'astra',
    text: 'can you review the patch?',
    timestamp: Date.now(),
    read: false,
    muted: false,
    ...overrides,
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  desktop.notify.mockClear();
  inbox.current = { ...inbox.current, items: [] };
});

function view() {
  return render(
    <I18nextProvider i18n={i18n}>
      <InboxNotice />
    </I18nextProvider>,
  );
}

describe('新内容提醒', () => {
  it('窗口不在前台时把新的提及交给系统通知，回到前台后只显示应用内横幅', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const rendered = view();
    inbox.current = { ...inbox.current, items: [item('m1')] };
    rendered.rerender(
      <I18nextProvider i18n={i18n}>
        <InboxNotice />
      </I18nextProvider>,
    );
    await waitFor(() => {
      expect(desktop.notify).toHaveBeenCalledWith({
        title: 'game dev',
        body: 'astra: can you review the patch?',
      });
    });
    expect(screen.getByRole('status')).toHaveTextContent('1 new item');

    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    inbox.current = { ...inbox.current, items: [item('m1'), item('m2', { sender: 'claude' })] };
    rendered.rerender(
      <I18nextProvider i18n={i18n}>
        <InboxNotice />
      </I18nextProvider>,
    );
    await screen.findByText(/1 new item/u);
    expect(desktop.notify).toHaveBeenCalledTimes(1);
  });

  it('静音房间和启动时已有的内容都不打扰', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    // 启动时就在收件箱里的内容不算新。
    inbox.current = { ...inbox.current, items: [item('old')] };
    const rendered = view();
    inbox.current = { ...inbox.current, items: [item('old'), item('quiet', { muted: true })] };
    rendered.rerender(
      <I18nextProvider i18n={i18n}>
        <InboxNotice />
      </I18nextProvider>,
    );
    await new Promise((resolve) => {
      setTimeout(resolve, 20);
    });
    expect(desktop.notify).not.toHaveBeenCalled();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('多条新内容只报数量，单条长文截断到 140 个字符', () => {
    const many = systemNotificationSummary([
      item('a'),
      item('b', { room: { catalogId: 'c', roomId: '!o', name: 'other', direct: false } }),
    ]);
    expect(many).toEqual({ count: 2, room: null, sender: null, text: '' });
    const long = systemNotificationSummary([item('a', { text: 'x'.repeat(200) })]);
    expect(long.sender).toBe('astra');
    expect(Array.from(long.text)).toHaveLength(140);
    expect(long.text.endsWith('…')).toBe(true);
  });
});
