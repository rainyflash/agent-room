import {
  type EventTimeline,
  type MatrixClient,
  type MatrixEvent,
  type Room,
  type RoomMember,
  type RoomState,
} from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkLobbySource, matrixAgentStatusEventType } from './matrix-lobby-source';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

describe('MatrixSdkLobbySource', () => {
  it('只从已加入房间的当前状态生成传输快照', () => {
    const statusContent = { eventType: matrixAgentStatusEventType };
    const state = matrixState(statusContent);
    const room = matrixRoom(state);
    const client = matrixClient(room);
    const registry = new MatrixClientRegistry();
    registry.replace(client.value);
    const source = new MatrixSdkLobbySource(registry);

    expect(source.read('!public:agent-room.test')).toEqual({
      kind: 'ready',
      room: {
        joinedMemberIds: ['@a:agent-room.test', '@z:agent-room.test'],
        name: '公开大厅',
        roomId: '!public:agent-room.test',
        statusEvents: [
          {
            content: statusContent,
            sender: '@a:agent-room.test',
            stateKey: 'instance-a',
          },
        ],
        topic: '协作工作区',
      },
    });
  });

  it('客户端租约变化与同步活动会通知快照订阅者', () => {
    const registry = new MatrixClientRegistry();
    const first = matrixClient(matrixRoom(matrixState({})));
    const second = matrixClient(matrixRoom(matrixState({})));
    registry.replace(first.value);
    const source = new MatrixSdkLobbySource(registry);
    const listener = vi.fn();

    const unsubscribe = source.subscribe('!public:agent-room.test', listener);
    registry.replace(second.value);
    registry.refresh(second.value);
    unsubscribe();
    registry.refresh(second.value);

    expect(listener).toHaveBeenCalledTimes(2);
  });

  it('没有客户端与未加入房间时不会伪造空大厅', () => {
    const registry = new MatrixClientRegistry();
    const source = new MatrixSdkLobbySource(registry);

    expect(source.read('!public:agent-room.test')).toEqual({ kind: 'matrix-unavailable' });

    registry.replace(matrixClient(null).value);
    expect(source.read('!public:agent-room.test')).toEqual({ kind: 'room-not-joined' });
  });
});

function matrixState(statusContent: unknown): RoomState {
  const statusEvent = {
    getContent: () => statusContent,
    getSender: () => '@a:agent-room.test',
    getStateKey: () => 'instance-a',
  } as unknown as MatrixEvent;
  const topicEvent = {
    getContent: () => ({ topic: '  协作工作区  ' }),
  } as unknown as MatrixEvent;
  return {
    getStateEvents: (eventType: string, stateKey?: string) => {
      if (eventType === matrixAgentStatusEventType) {
        return [statusEvent];
      }
      return eventType === 'm.room.topic' && stateKey === '' ? topicEvent : null;
    },
    roomId: '!public:agent-room.test',
  } as unknown as RoomState;
}

function matrixRoom(state: RoomState): Room {
  const timeline = {
    getState: () => state,
  } as unknown as EventTimeline;
  return {
    getJoinedMembers: () =>
      [{ userId: '@z:agent-room.test' }, { userId: '@a:agent-room.test' }] as RoomMember[],
    getLiveTimeline: () => timeline,
    name: '  公开大厅  ',
  } as unknown as Room;
}

function matrixClient(room: Room | null): {
  readonly value: MatrixClient;
} {
  return {
    value: {
      getRoom: () => room,
    } as unknown as MatrixClient,
  };
}
