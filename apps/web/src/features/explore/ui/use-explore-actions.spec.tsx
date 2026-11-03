// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ExploreRoom } from '@/features/explore/domain/explore';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { err, ok } from '@/shared/result';

const commands = { enter: vi.fn(), decline: vi.fn(), create: vi.fn() };
const services = {
  privateRooms: { inspect: vi.fn() },
  publicRoomEntry: { join: vi.fn(), requestToJoin: vi.fn(), withdrawJoinRequest: vi.fn() },
};

vi.mock('@/app/app-services', () => ({ useAppServices: () => services }));
vi.mock('@/features/room-directory/ui/use-rooms', () => ({ useRoomCommands: () => commands }));

const { useExploreActions } = await import('./use-explore-actions');

const room: ExploreRoom = {
  agentCount: 1,
  catalogId: '0198c000-0000-7000-8000-000000000001',
  createdAtUnixMs: 1_792_483_200_000,
  description: '',
  endToEndEncrypted: false,
  joinMode: 'open',
  lastMessageAtUnixMs: null,
  memberCount: 2,
  name: 'Rust help desk',
  ownerName: 'Grace',
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
  viewer: null,
};
const inspected = { catalogId: room.catalogId, name: room.name } as unknown as PrivateRoom;

function wrapper({ children }: { readonly children: ReactNode }) {
  return <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  vi.resetAllMocks();
  services.privateRooms.inspect.mockResolvedValue(ok(inspected));
  commands.enter.mockResolvedValue(ok(inspected));
});

describe('从发现页进公开房间', () => {
  it('直接进：服务器邀请了你，照接受邀请那一套进房间', async () => {
    services.publicRoomEntry.join.mockResolvedValue(
      ok({ catalogId: room.catalogId, membership: 'invited' }),
    );
    const { result } = renderHook(() => useExploreActions(), { wrapper });

    act(() => {
      result.current.onJoin(room);
    });
    expect(result.current.busy).toEqual({ catalogId: room.catalogId, kind: 'join' });
    await waitFor(() => {
      expect(result.current.busy).toBeNull();
    });
    expect(services.privateRooms.inspect).toHaveBeenCalledWith(room.catalogId);
    expect(commands.enter).toHaveBeenCalledWith(inspected, true);
    expect(result.current.failure).toBeNull();
  });

  it('收到过邀请的直接接受，不再问服务器', async () => {
    const { result } = renderHook(() => useExploreActions(), { wrapper });

    act(() => {
      result.current.onJoin({ ...room, joinMode: 'invite_only', viewer: 'invited' });
    });
    await waitFor(() => {
      expect(commands.enter).toHaveBeenCalledWith(inspected, true);
    });
    expect(services.publicRoomEntry.join).not.toHaveBeenCalled();
  });

  it('没进成时按房间记下错误码', async () => {
    services.publicRoomEntry.join.mockResolvedValue(
      err({ code: 'room_join.approval_required', retryable: false }),
    );
    const { result } = renderHook(() => useExploreActions(), { wrapper });

    act(() => {
      result.current.onJoin(room);
    });
    await waitFor(() => {
      expect(result.current.failure).toEqual({
        catalogId: room.catalogId,
        code: 'room_join.approval_required',
      });
    });
    expect(commands.enter).not.toHaveBeenCalled();
  });

  it('申请带上去掉首尾空白的留言，在等批准时不进房间；撤回自己的申请', async () => {
    services.publicRoomEntry.requestToJoin.mockResolvedValue(
      ok({ catalogId: room.catalogId, membership: 'pending', requestedAtUnixMs: 1 }),
    );
    services.publicRoomEntry.withdrawJoinRequest.mockResolvedValue(ok(undefined));
    const { result } = renderHook(() => useExploreActions(), { wrapper });

    act(() => {
      result.current.onRequest({ ...room, joinMode: 'request' }, '  hello\n');
    });
    await waitFor(() => {
      expect(result.current.busy).toBeNull();
    });
    expect(services.publicRoomEntry.requestToJoin).toHaveBeenCalledWith(room.catalogId, 'hello');
    expect(commands.enter).not.toHaveBeenCalled();

    act(() => {
      result.current.onWithdraw({ ...room, viewer: 'pending' });
    });
    await waitFor(() => {
      expect(services.publicRoomEntry.withdrawJoinRequest).toHaveBeenCalledWith(room.catalogId);
    });
  });
});
