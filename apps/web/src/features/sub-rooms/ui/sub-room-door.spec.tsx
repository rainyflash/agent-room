// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const commands = { enter: vi.fn(), decline: vi.fn(), create: vi.fn(), archive: vi.fn() };
const services = {
  privateRooms: { inspect: vi.fn(), people: vi.fn() },
  subRooms: {
    join: vi.fn(),
    requestToJoin: vi.fn(),
    restore: vi.fn(),
    withdrawJoinRequest: vi.fn(),
  },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));
vi.mock('@/features/room-directory/ui/use-rooms', () => ({ useRoomCommands: () => commands }));
vi.mock('@/features/inbox/ui/room-unread-mark', () => ({ useCatalogUnread: () => undefined }));
vi.mock('@tanstack/react-router', () => ({
  Link: ({
    children,
    params,
    to,
    ...rest
  }: {
    readonly children: ReactNode;
    readonly params: { readonly catalogId: string };
    readonly to: string;
  }) => (
    <a href={to.replace('$catalogId', params.catalogId)} {...rest}>
      {children}
    </a>
  ),
}));

const { SubRoomDoor } = await import('./sub-room-door');

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e40';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

function subRoom(extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e55',
    name: 'Backup rework',
    description: 'Off-site copies and restore drills.',
    access: 'request',
    hidden: false,
    createdByPrincipalId: GRACE,
    createdAtUnixMs: 1,
    managerPrincipalIds: [GRACE],
    categoryId: OPS,
    joined: false,
    banned: false,
    ...extra,
  };
}

function list(room: SubRoom): SubRoomList {
  return {
    parent: { catalogId: PARENT, name: 'Agent Room' },
    canCreate: false,
    categories: [{ categoryId: OPS, name: 'Ops', position: 0 }],
    subRooms: [room],
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  vi.resetAllMocks();
  services.privateRooms.people.mockResolvedValue(
    ok([{ displayName: 'Grace', principalId: GRACE }]),
  );
});

afterEach(() => {
  cleanup();
});

function renderDoor(room: SubRoom) {
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <SubRoomDoor list={list(room)} room={room} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

describe('还没进的子房间的卡片', () => {
  it('写分类和门、用途、谁在管；要批准的申请加入，留句话发出去', async () => {
    services.subRooms.requestToJoin.mockResolvedValue(
      ok({ catalogId: subRoom().catalogId, membership: 'pending', requestedAtUnixMs: 5 }),
    );
    renderDoor(subRoom());

    expect(screen.getByRole('heading', { level: 2, name: 'Backup rework' })).toBeVisible();
    expect(screen.getByText('Ops · Needs approval to join')).toBeVisible();
    expect(screen.getByText('Off-site copies and restore drills.')).toBeVisible();
    expect(await screen.findByText('Grace runs this one')).toBeVisible();
    expect(screen.getByText('Grace or the room’s admins can let you in.')).toBeVisible();
    expect(screen.getByText(/Your agents can come in once you’re in/u)).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Ask to join' }));
    const form = screen.getByRole('form', { name: 'Ask to join Backup rework' });
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'On call' } });
    fireEvent.click(screen.getByRole('button', { name: 'Send request' }));

    await waitFor(() => {
      expect(services.subRooms.requestToJoin).toHaveBeenCalledWith(
        { parentId: PARENT, subRoomId: subRoom().catalogId },
        'On call',
      );
    });
    expect(form).toBeInTheDocument();
  });

  it('在等批准：写在等谁，能撤回', async () => {
    services.subRooms.withdrawJoinRequest.mockResolvedValue(ok(undefined));
    renderDoor(subRoom({ requestedAtUnixMs: 5 }));

    expect(await screen.findByText('Waiting for Grace to approve.')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Withdraw request' }));
    await waitFor(() => {
      expect(services.subRooms.withdrawJoinRequest).toHaveBeenCalledWith({
        parentId: PARENT,
        subRoomId: subRoom().catalogId,
      });
    });
  });

  it('只能请进来的没有按钮；没写管理员的说房间的房主和管理员在管', () => {
    renderDoor(subRoom({ access: 'invite_only', managerPrincipalIds: [] }));

    expect(screen.getByText('The room’s owner and admins run this one')).toBeVisible();
    expect(
      screen.getByText('Only an admin can bring people in here. You can’t ask to join.'),
    ).toBeVisible();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('没有门的直接加入，加入以后照进私人房间那样进去', async () => {
    const joined = { catalogId: subRoom().catalogId } as unknown as PrivateRoom;
    services.subRooms.join.mockResolvedValue(ok(joined));
    commands.enter.mockResolvedValue(ok(joined));
    renderDoor(subRoom({ access: 'open' }));

    expect(screen.getByText('Ops · Anyone in the room can walk in')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Join this one' }));
    await waitFor(() => {
      expect(commands.enter).toHaveBeenCalledWith(joined, false);
    });
  });

  it('归档了的：写已经归档，原来在里面的能打开，能恢复的有“恢复”', async () => {
    services.subRooms.restore.mockResolvedValue(ok(undefined));
    renderDoor(subRoom({ archivedAtUnixMs: 5, canRestore: true, joined: true }));

    expect(
      screen.getByText(
        'This sub-room is archived. Earlier messages can still be read, and nobody can post.',
      ),
    ).toBeVisible();
    expect(screen.getByRole('link', { name: 'Open' })).toHaveAttribute(
      'href',
      `/lobby/${subRoom().catalogId}`,
    );
    expect(screen.queryByText(/Your agents can come in/u)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    await waitFor(() => {
      expect(services.subRooms.restore).toHaveBeenCalledOnce();
    });
  });
});
