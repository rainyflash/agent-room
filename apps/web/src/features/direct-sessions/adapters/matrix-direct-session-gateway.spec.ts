import { EventType, ReceiptType, type MatrixClient, type MatrixEvent, type Room } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkDirectSessionGateway } from './matrix-direct-session-gateway';
import type { DirectSession } from '@/features/direct-sessions/domain/direct-session';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

describe('MatrixSdkDirectSessionGateway', () => {
  it('加入邀请房间并保留已有的 m.direct 映射', async () => {
    const joinedRoom = room('join');
    const invitedRoom = room('invite');
    const joinRoom = vi.fn().mockResolvedValue(joinedRoom);
    const setAccountData = vi.fn().mockResolvedValue({});
    const client = {
      getAccountDataFromServer: vi.fn().mockResolvedValue({
        '@existing:matrix.agent-room.test': ['!existing:matrix.agent-room.test'],
      }),
      getRoom: vi.fn().mockReturnValue(invitedRoom),
      joinRoom,
      setAccountData,
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkDirectSessionGateway(source(client));

    await expect(gateway.prepare(session())).resolves.toEqual({ ok: true, value: undefined });
    expect(joinRoom).toHaveBeenCalledWith('!direct:matrix.agent-room.test');
    expect(setAccountData).toHaveBeenCalledWith(EventType.Direct, {
      '@_agent_build:matrix.agent-room.test': ['!direct:matrix.agent-room.test'],
      '@existing:matrix.agent-room.test': ['!existing:matrix.agent-room.test'],
    });
  });

  it('忽略列表更新幂等且只私下发送已展示事件回执', async () => {
    const event = {} as MatrixEvent;
    const setIgnoredUsers = vi.fn().mockResolvedValue({});
    const sendReadReceipt = vi.fn().mockResolvedValue({});
    const client = {
      getIgnoredUsers: vi.fn().mockReturnValue(['@existing:matrix.agent-room.test']),
      getRoom: vi.fn().mockReturnValue({
        findEventById: vi.fn().mockReturnValue(event),
      }),
      sendReadReceipt,
      setIgnoredUsers,
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkDirectSessionGateway(source(client));

    await expect(
      gateway.setIgnored('@_agent_build:matrix.agent-room.test', true),
    ).resolves.toEqual({ ok: true, value: undefined });
    expect(setIgnoredUsers).toHaveBeenCalledWith([
      '@existing:matrix.agent-room.test',
      '@_agent_build:matrix.agent-room.test',
    ]);

    await expect(gateway.markDisplayed('!direct:matrix.agent-room.test', '$event')).resolves.toEqual(
      { ok: true, value: undefined },
    );
    expect(sendReadReceipt).toHaveBeenCalledWith(event, ReceiptType.ReadPrivate, true);
  });
});

function room(membership: 'invite' | 'join'): Room {
  return { getMyMembership: () => membership } as unknown as Room;
}

function source(client: MatrixClient | null): MatrixClientSource {
  return {
    current: () => client,
    subscribe: () => () => undefined,
  };
}

function session(): DirectSession {
  return {
    catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e53',
    contactPolicy: {
      agentBlocksPrincipal: false,
      deliveryAllowed: true,
      presenceDisclosure: 'coarse',
      principalBlocksAgent: false,
    },
    lifecycle: 'active',
    matrixRoomId: '!direct:matrix.agent-room.test',
    roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e54',
    target: {
      agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e52',
      avatarContentId: null,
      displayName: 'Build Agent',
      matrixUserId: '@_agent_build:matrix.agent-room.test',
    },
    version: 1,
  };
}
