// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { InboxNotice } from './inbox-notice';
import { WebNotificationsProvider } from './web-notifications-provider';
import type { WebNotificationState } from '../adapters/web-notifications';
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
    rooms: new Map(),
    limited: false,
    unavailableRooms: 0,
    status: 'ready' as const,
  },
}));
const navigate = vi.hoisted(() => vi.fn(() => Promise.resolve()));
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
  useNavigate: () => navigate,
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
  desktop.available = true;
  navigate.mockClear();
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

function fakeNotifications(initial: Partial<WebNotificationState> = {}) {
  let state: WebNotificationState = {
    offered: false,
    permission: 'default',
    wanted: false,
    ...initial,
  };
  return {
    markOffered: vi.fn(() => {
      state = { ...state, offered: true };
    }),
    refresh: vi.fn(),
    setWanted: vi.fn((wanted: boolean) => {
      state = { ...state, permission: 'granted', wanted };
      return Promise.resolve(state);
    }),
    show: vi.fn((note: { readonly onClick: () => void }) => {
      void note;
      return true;
    }),
    state: () => state,
    subscribe: () => () => undefined,
  };
}

function webTree(notifications: ReturnType<typeof fakeNotifications>) {
  return (
    <I18nextProvider i18n={i18n}>
      <WebNotificationsProvider value={notifications}>
        <InboxNotice />
      </WebNotificationsProvider>
    </I18nextProvider>
  );
}

describe('网页通知（4R-2）', () => {
  it('网页上标签页在后台时弹浏览器通知，点了回到那一条', async () => {
    desktop.available = false;
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const focus = vi.spyOn(window, 'focus').mockImplementation(() => undefined);
    const notifications = fakeNotifications({ offered: true, permission: 'granted', wanted: true });
    const rendered = render(webTree(notifications));
    inbox.current = { ...inbox.current, items: [item('m1')] };
    rendered.rerender(webTree(notifications));
    await waitFor(() => {
      expect(notifications.show).toHaveBeenCalledTimes(1);
    });
    const note = notifications.show.mock.calls[0]?.[0] as {
      readonly title: string;
      readonly body: string;
      readonly tag: string;
      readonly onClick: () => void;
    };
    expect(note).toMatchObject({
      body: 'astra: can you review the patch?',
      tag: 'agent-room-room:game dev',
      title: 'game dev',
    });
    note.onClick();
    expect(focus).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith({
      params: { catalogId: 'catalog', roomId: '!room:test' },
      search: { message: 'm1', view: 'conversation' },
      to: '/lobby/$catalogId/instance/$roomId',
    });
    expect(desktop.notify).not.toHaveBeenCalled();
    // 已经开着，不再问。
    expect(screen.queryByText('Get these in the background too?')).not.toBeInTheDocument();
  });

  it('标签页在前面时不弹，只有页面里的提示', async () => {
    desktop.available = false;
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const notifications = fakeNotifications({ offered: true, permission: 'granted', wanted: true });
    const rendered = render(webTree(notifications));
    inbox.current = { ...inbox.current, items: [item('m1')] };
    rendered.rerender(webTree(notifications));
    await screen.findByText(/1 new item/u);
    expect(notifications.show).not.toHaveBeenCalled();
  });

  it('第一次有人找你、还没开过网页通知时问一句；打开以后记下问过了', async () => {
    desktop.available = false;
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const notifications = fakeNotifications();
    const rendered = render(webTree(notifications));
    inbox.current = { ...inbox.current, items: [item('m1')] };
    rendered.rerender(webTree(notifications));
    await screen.findByText('Get these in the background too?');

    fireEvent.click(screen.getByRole('button', { name: 'Turn on web notifications' }));
    await waitFor(() => {
      expect(notifications.markOffered).toHaveBeenCalledTimes(1);
    });
    expect(notifications.setWanted).toHaveBeenCalledWith(true);
    expect(screen.queryByText('Get these in the background too?')).not.toBeInTheDocument();
  });

  it('“不用了”也算答过；浏览器拒过的、桌面端都不问', async () => {
    desktop.available = false;
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const notifications = fakeNotifications();
    const rendered = render(webTree(notifications));
    inbox.current = { ...inbox.current, items: [item('m1')] };
    rendered.rerender(webTree(notifications));
    fireEvent.click(await screen.findByRole('button', { name: 'No thanks' }));
    expect(notifications.markOffered).toHaveBeenCalledTimes(1);
    expect(notifications.setWanted).not.toHaveBeenCalled();
    cleanup();

    const denied = fakeNotifications({ permission: 'denied' });
    inbox.current = { ...inbox.current, items: [] };
    const again = render(webTree(denied));
    inbox.current = { ...inbox.current, items: [item('m2')] };
    again.rerender(webTree(denied));
    await screen.findByText(/1 new item/u);
    expect(screen.queryByText('Get these in the background too?')).not.toBeInTheDocument();
    cleanup();

    desktop.available = true;
    const onDesktop = fakeNotifications();
    inbox.current = { ...inbox.current, items: [] };
    const desk = render(webTree(onDesktop));
    inbox.current = { ...inbox.current, items: [item('m3')] };
    desk.rerender(webTree(onDesktop));
    await screen.findByText(/1 new item/u);
    expect(screen.queryByText('Get these in the background too?')).not.toBeInTheDocument();
    expect(onDesktop.show).not.toHaveBeenCalled();
  });
});
