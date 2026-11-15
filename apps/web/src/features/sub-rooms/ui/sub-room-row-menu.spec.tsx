// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomUnread } from '@/features/inbox/domain/inbox';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { SubRoomPlaces, type SubRoomPlacesProps } from './sub-room-places';
import type { SubRoomRowMenuContext } from './sub-room-row-menu';

const unread = new Map<string, { roomId: string; unread: RoomUnread }>();
const muted = new Set<string>();
const change = vi.fn();
const navigate = vi.fn();
const services = {
  messages: { read: vi.fn(), subscribe: vi.fn() },
  subRooms: { archive: vi.fn(), leave: vi.fn(), revise: vi.fn() },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));
vi.mock('@/features/inbox/ui/room-unread-mark', () => ({
  useCatalogUnread: (catalogId: string) => unread.get(catalogId),
}));
vi.mock('@/features/personal-workspace/ui/personal-workspace-provider', () => ({
  usePersonalWorkspace: () => ({ change, snapshot: { index: { muted } } }),
}));
vi.mock('@tanstack/react-router', () => ({
  Link: ({
    children,
    params,
    to,
    ...rest
  }: {
    readonly children: ReactNode;
    readonly params: { readonly catalogId: string; readonly roomId?: string };
    readonly to: string;
  }) => (
    <a
      href={to.replace('$catalogId', params.catalogId).replace('$roomId', params.roomId ?? '')}
      {...rest}
    >
      {children}
    </a>
  ),
  useNavigate: () => navigate,
}));

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const ME = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

function subRoom(id: string, name: string, extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${id}`,
    name,
    description: '',
    access: 'open',
    hidden: false,
    createdByPrincipalId: ME,
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    joined: true,
    banned: false,
    ...extra,
  };
}

const RELEASE = subRoom('51', 'Release', { categoryId: WORK });
const BACKUPS = subRoom('52', 'Backups', { access: 'request' });

function list(rooms: readonly SubRoom[]): SubRoomList {
  return {
    parent: { catalogId: PARENT, name: 'Agent Room' },
    canCreate: true,
    categories: [
      { categoryId: OPS, name: 'Ops', position: 1 },
      { categoryId: WORK, name: 'Work', position: 0 },
    ],
    subRooms: [...rooms],
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  change.mockReturnValue(ok(undefined));
  unread.set(RELEASE.catalogId, {
    roomId: '!release:example.org',
    unread: { catalogId: RELEASE.catalogId, count: 3, more: false, mentioned: true },
  });
  unread.set(BACKUPS.catalogId, {
    roomId: '!backups:example.org',
    unread: { catalogId: BACKUPS.catalogId, count: 0, more: false, mentioned: false },
  });
});

afterEach(() => {
  cleanup();
  unread.clear();
  muted.clear();
  window.localStorage.clear();
});

function renderPlaces(
  rooms: readonly SubRoom[],
  menu: Partial<SubRoomRowMenuContext> = {},
  overrides: Partial<SubRoomPlacesProps> = {},
) {
  const context: SubRoomRowMenuContext = {
    principalId: ME,
    ownerPrincipalId: OWNER,
    parentMatrixRoomId: '!parent:example.org',
    onOpenSettings: vi.fn(),
    ...menu,
  };
  const props: SubRoomPlacesProps = {
    browse: null,
    currentCatalogId: PARENT,
    currentSelected: true,
    currentUnread: 0,
    list: list(rooms),
    menu: context,
    onCreate: null,
    onNavigate: vi.fn(),
    onSelectCurrent: vi.fn(),
    ...overrides,
  };
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <SubRoomPlaces {...props} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return { context, props };
}

function openMenu(name: string) {
  fireEvent.click(screen.getByRole('button', { name: `Options for ${name}` }));
  return screen.getByRole('dialog', { name: `Options for ${name}` });
}

const itemNames = (menu: HTMLElement) =>
  within(menu)
    .getAllByRole('button')
    .map((button) => button.textContent);

describe('左栏那一行的“⋯”', () => {
  it('在里面的人：标为已读、免打扰、退出；标为已读挪到最新一条', () => {
    const latest = conversationFixture('latest', { serverTimestamp: 30 });
    services.messages.read.mockReturnValue(
      ok({
        roomId: '!release:example.org',
        messages: [conversationFixture('older', { serverTimestamp: 10 }), latest],
        observedAtUnixMs: 0,
        readOnlyFederatedEvents: [],
      }),
    );
    renderPlaces([RELEASE]);

    const menu = openMenu('Release');
    expect(itemNames(menu)).toEqual(['Mark as read', 'MuteMentions still show', 'Leave']);
    fireEvent.click(within(menu).getByRole('button', { name: 'Mark as read' }));

    expect(services.messages.read).toHaveBeenCalledWith('!release:example.org');
    expect(change).toHaveBeenCalledWith({
      kind: 'read',
      id: '!release:example.org',
      value: { eventId: latest.matrixEventId, timestamp: 30 },
    });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('没有没读的时标为已读点不了；免打扰写进这个人的设置，免打扰的只剩提到你的', () => {
    renderPlaces([RELEASE, BACKUPS]);

    const menu = openMenu('Backups');
    expect(within(menu).getByRole('button', { name: 'Mark as read' })).toBeDisabled();
    fireEvent.click(within(menu).getByRole('button', { name: /^Mute/u }));
    expect(change).toHaveBeenCalledWith({ kind: 'mute', id: '!backups:example.org', value: true });

    cleanup();
    muted.add('!release:example.org');
    renderPlaces([RELEASE]);
    const release = screen.getByRole('link', { name: 'Release' });
    expect(release).toHaveAccessibleDescription('Muted Mentions you');
    // 3 条没读的不数了，红底的只写 @。
    expect(release).toHaveTextContent('Muted@Mentions you');
    expect(release).not.toHaveTextContent('3');
    expect(itemNames(openMenu('Release'))).toContain('Unmute');
  });

  it('父房间的房主退不出去，菜单里没有“退出”', () => {
    renderPlaces([RELEASE], { ownerPrincipalId: ME });

    expect(itemNames(openMenu('Release'))).not.toContain('Leave');
  });

  it('退出眼前这一间：先问一句，退了回到父房间；服务器拒了照实说', async () => {
    services.subRooms.leave.mockResolvedValueOnce(
      err({ code: 'sub_room.owner_stays', retryable: false }),
    );
    services.subRooms.leave.mockResolvedValueOnce(ok(undefined));
    const { props } = renderPlaces([BACKUPS], {}, { currentCatalogId: BACKUPS.catalogId });

    const menu = openMenu('Backups');
    fireEvent.click(within(menu).getByRole('button', { name: 'Leave' }));
    const confirm = within(menu).getByRole('group', { name: 'Leave “Backups”?' });
    expect(confirm).toHaveTextContent('To come back, you’ll have to ask again.');
    expect(within(confirm).getByRole('button', { name: 'Cancel' })).toHaveFocus();

    fireEvent.click(within(confirm).getByRole('button', { name: 'Leave' }));
    expect(await within(menu).findByRole('alert')).toHaveTextContent(
      'The room’s owner is in every sub-room and can’t leave.',
    );
    fireEvent.click(within(confirm).getByRole('button', { name: 'Leave' }));
    await waitFor(() => {
      expect(navigate).toHaveBeenCalledWith({
        params: { catalogId: PARENT, roomId: '!parent:example.org' },
        to: '/lobby/$catalogId/instance/$roomId',
      });
    });
    expect(services.subRooms.leave).toHaveBeenCalledWith({
      parentId: PARENT,
      subRoomId: BACKUPS.catalogId,
    });
    expect(props.onNavigate).toHaveBeenCalled();
  });

  it('管这一间的人：移到分类在菜单里换成分类的名单，“未分类”是不放进分类', async () => {
    services.subRooms.revise.mockResolvedValue(ok({}));
    const managed = { ...RELEASE, pendingRequestCount: 0 };
    renderPlaces([managed]);

    let menu = openMenu('Release');
    expect(itemNames(menu)).toEqual([
      'Mark as read',
      'MuteMentions still show',
      'Move to category',
      'Sub-room settings',
      'Leave',
      'Archive',
    ]);
    fireEvent.click(within(menu).getByRole('button', { name: 'Move to category' }));
    expect(menu).toHaveTextContent('Move “Release” to');
    expect(within(menu).getByRole('button', { name: 'Work' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(within(menu).getByRole('button', { name: 'Work' })).toHaveFocus();
    fireEvent.click(within(menu).getByRole('button', { name: 'Back' }));
    expect(within(menu).getByRole('button', { name: 'Move to category' })).toHaveFocus();

    fireEvent.click(within(menu).getByRole('button', { name: 'Move to category' }));
    fireEvent.click(within(menu).getByRole('button', { name: 'Uncategorized' }));
    await waitFor(() => {
      expect(services.subRooms.revise).toHaveBeenCalledWith(
        { parentId: PARENT, subRoomId: RELEASE.catalogId },
        { categoryId: null },
      );
    });
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });

    menu = openMenu('Release');
    fireEvent.click(within(menu).getByRole('button', { name: 'Move to category' }));
    fireEvent.click(within(menu).getByRole('button', { name: 'Ops' }));
    await waitFor(() => {
      expect(services.subRooms.revise).toHaveBeenLastCalledWith(
        { parentId: PARENT, subRoomId: RELEASE.catalogId },
        { categoryId: OPS },
      );
    });
  });

  it('管这一间的人：设置交给房间页打开，归档先问一句', async () => {
    services.subRooms.archive.mockResolvedValue(ok(undefined));
    const managed = { ...RELEASE, pendingRequestCount: 0 };
    const { context, props } = renderPlaces([managed]);

    fireEvent.click(within(openMenu('Release')).getByRole('button', { name: 'Sub-room settings' }));
    expect(context.onOpenSettings).toHaveBeenCalledWith(managed);
    expect(props.onNavigate).toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    const menu = openMenu('Release');
    fireEvent.click(within(menu).getByRole('button', { name: 'Archive' }));
    const confirm = within(menu).getByRole('group', { name: 'Archive “Release”?' });
    fireEvent.click(within(confirm).getByRole('button', { name: 'Archive' }));
    await waitFor(() => {
      expect(services.subRooms.archive).toHaveBeenCalledWith({
        parentId: PARENT,
        subRoomId: RELEASE.catalogId,
      });
    });
    expect(navigate).not.toHaveBeenCalled();
  });
});
