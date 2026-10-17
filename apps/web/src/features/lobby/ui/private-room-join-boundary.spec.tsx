// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import type {
  PrivateRoom,
  PrivateRoomGateway,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PRIVATE_ROOM_JOIN_SETTLE_MS, PrivateRoomJoinBoundary } from './private-room-join-boundary';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GUEST = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const ROOM = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: '',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    { permissions: { capabilities: ['view', 'speak'] }, principalId: GUEST, status: 'invited' },
  ],
  name: 'Sync probe',
  ownerPrincipalId: OWNER,
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 0,
} satisfies PrivateRoom;
const NOT_JOINED = { code: 'lobby.room_not_joined', kind: 'failed', retryable: true } as const;

function renderBoundary({
  join = vi.fn<PrivateRoomMatrixGateway['join']>(() => Promise.resolve(ok(undefined))),
  principalId = OWNER,
  rooms = [ROOM],
}: {
  readonly join?: ReturnType<typeof vi.fn<PrivateRoomMatrixGateway['join']>>;
  readonly principalId?: string | null;
  readonly rooms?: readonly PrivateRoom[];
} = {}) {
  const onRetry = vi.fn();
  const services = {
    privateRoomMatrix: { join, leave: vi.fn() } satisfies PrivateRoomMatrixGateway,
    privateRooms: { list: () => Promise.resolve(ok(rooms)) } as unknown as PrivateRoomGateway,
  } as unknown as AppServices;
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <AppServicesProvider services={services}>
          <PrivateRoomJoinBoundary
            catalogId={ROOM.catalogId}
            onRetry={onRetry}
            principalId={principalId}
            roomId={ROOM.matrixRoomId}
            state={NOT_JOINED}
          />
        </AppServicesProvider>
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return { join, onRetry };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('打开 Matrix 上还没加入的私人房间', () => {
  it('服务器记着你是房主：直接替你加入，一直显示正在打开，不叫你回列表找“加入”', async () => {
    const { join } = renderBoundary();

    await waitFor(() => {
      expect(join).toHaveBeenCalledWith(ROOM.matrixRoomId);
    });
    expect(join).toHaveBeenCalledTimes(1);
    expect(screen.getByText('Opening the room')).toBeVisible();
    expect(screen.queryByText('You are not in this room yet')).not.toBeInTheDocument();
  });

  it('还没答复的邀请不替你加入：照旧说先在房间列表里加入', async () => {
    const { join } = renderBoundary({ principalId: GUEST });

    expect(await screen.findByText('You are not in this room yet')).toBeVisible();
    expect(join).not.toHaveBeenCalled();
  });

  it('不在你的私人房间里（公共大厅、别人的房间）也不加入', async () => {
    const { join } = renderBoundary({ rooms: [] });

    expect(await screen.findByText('You are not in this room yet')).toBeVisible();
    expect(join).not.toHaveBeenCalled();
  });

  it('加入没成就说打不开；“重试”再加入一次，也让房间重读', async () => {
    const join = vi
      .fn<PrivateRoomMatrixGateway['join']>()
      .mockResolvedValueOnce(err({ code: 'private_room.matrix_join_failed', retryable: true }))
      .mockResolvedValue(ok(undefined));
    const { onRetry } = renderBoundary({ join });

    fireEvent.click(await screen.findByRole('button', { name: 'Try again' }));

    await waitFor(() => {
      expect(join).toHaveBeenCalledTimes(2);
    });
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(await screen.findByText('Opening the room')).toBeVisible();
  });

  it('加入了却迟迟没同步回来：等一阵以后照常说打不开，能重试', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { join } = renderBoundary();
    await waitFor(() => {
      expect(join).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByText('Opening the room')).toBeVisible();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(PRIVATE_ROOM_JOIN_SETTLE_MS);
    });

    expect(screen.getByText('You are not in this room yet')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeVisible();
  });
});
