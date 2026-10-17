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
import type { AgentPresenceSource } from './matrix-presence-tracker';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

/** 这台设备上登录的人。 */
const SELF = '@self:agent-room.test';

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
        canSpeak: true,
        encrypted: false,
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

  it('房间有加密状态事件时标为加密', () => {
    const registry = new MatrixClientRegistry();
    registry.replace(matrixClient(matrixRoom(matrixState({}, true))).value);
    const read = new MatrixSdkLobbySource(registry).read('!public:agent-room.test');

    expect(read.kind === 'ready' && read.room.encrypted).toBe(true);
  });

  it('私人房间带上已加入成员的 Matrix 显示名，按用户 ID 排好；公开大厅不带', () => {
    const named = (encrypted: boolean) => {
      const registry = new MatrixClientRegistry();
      registry.replace(matrixClient(matrixRoom(matrixState({}, encrypted))).value);
      const read = new MatrixSdkLobbySource(registry).read('!public:agent-room.test');
      return read.kind === 'ready' ? read.room.memberNames : 'not ready';
    };

    expect(named(true)).toEqual([
      { displayName: 'Name of @a:agent-room.test', matrixUserId: '@a:agent-room.test' },
      { displayName: 'Name of @z:agent-room.test', matrixUserId: '@z:agent-room.test' },
    ]);
    expect(named(false)).toBeUndefined();
  });

  it('按 Matrix 的发言门槛说这台设备上的人能不能在这里发消息', () => {
    const canSpeak = (state: RoomState, self: string | null = SELF) => {
      const registry = new MatrixClientRegistry();
      registry.replace(matrixClient(matrixRoom(state), self).value);
      const read = new MatrixSdkLobbySource(registry).read('!public:agent-room.test');
      return read.kind === 'ready' && read.room.canSpeak;
    };

    expect(canSpeak(matrixState({}))).toBe(true);
    // 房间归档、被禁言或只能查看：发言门槛高过他的级别。
    expect(canSpeak(matrixState({}, false, []))).toBe(false);
    expect(canSpeak(matrixState({}), null)).toBe(false);
  });

  it.each(['invite', 'leave', 'ban'])('缓存中的 %s 房间不能继续显示在场人物', (membership) => {
    const registry = new MatrixClientRegistry();
    registry.replace(matrixClient(matrixRoom(matrixState({}), membership)).value);
    expect(new MatrixSdkLobbySource(registry).read('!public:agent-room.test')).toEqual({
      kind: 'room-not-joined',
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

  it('只为还在房间里、写名片的 Agent 读在线状态', () => {
    const card = { eventType: matrixAgentStatusEventType, liveness: 'presence' };
    const registry = new MatrixClientRegistry();
    registry.replace(matrixClient(matrixRoom(matrixState(card))).value);
    const presence = presenceSource();

    const read = new MatrixSdkLobbySource(registry, presence).read('!public:agent-room.test');

    expect(presence.observe).toHaveBeenCalledWith(['@a:agent-room.test']);
    expect(read.kind === 'ready' && read.room.presence).toEqual(
      new Map([['@a:agent-room.test', { state: 'online' }]]),
    );
  });

  it('旧的租约写法和已经离开房间的人不读在线状态', () => {
    const lease = { eventType: matrixAgentStatusEventType };
    const card = { eventType: matrixAgentStatusEventType, liveness: 'presence' };
    const presence = presenceSource();
    for (const room of [
      matrixRoom(matrixState(lease)),
      matrixRoom(matrixState(card), 'join', ['@z:agent-room.test']),
    ]) {
      const registry = new MatrixClientRegistry();
      registry.replace(matrixClient(room).value);
      const read = new MatrixSdkLobbySource(registry, presence).read('!public:agent-room.test');
      expect(read.kind === 'ready' && read.room.presence).toBeUndefined();
    }
    expect(presence.observe).not.toHaveBeenCalled();
  });

  it('问到在线状态时也通知快照订阅者', () => {
    const registry = new MatrixClientRegistry();
    const presence = presenceSource();
    const source = new MatrixSdkLobbySource(registry, presence);
    const listener = vi.fn();

    const unsubscribe = source.subscribe('!public:agent-room.test', listener);
    presence.notify();
    unsubscribe();
    presence.notify();

    expect(listener).toHaveBeenCalledOnce();
  });

  it('没有客户端与未加入房间时不会伪造空大厅', () => {
    const registry = new MatrixClientRegistry();
    const source = new MatrixSdkLobbySource(registry);

    expect(source.read('!public:agent-room.test')).toEqual({ kind: 'matrix-unavailable' });

    registry.replace(matrixClient(null).value);
    expect(source.read('!public:agent-room.test')).toEqual({ kind: 'room-not-joined' });
  });
});

function matrixState(
  statusContent: unknown,
  encrypted = false,
  speakers: readonly string[] = [SELF],
): RoomState {
  const statusEvent = {
    getContent: () => statusContent,
    getSender: () => '@a:agent-room.test',
    getStateKey: () => 'instance-a',
  } as unknown as MatrixEvent;
  const topicEvent = {
    getContent: () => ({ topic: '  协作工作区  ' }),
  } as unknown as MatrixEvent;
  const encryptionEvent = {
    getContent: () => ({ algorithm: 'm.megolm.v1.aes-sha2' }),
  } as unknown as MatrixEvent;
  return {
    getStateEvents: (eventType: string, stateKey?: string) => {
      if (eventType === matrixAgentStatusEventType) {
        return [statusEvent];
      }
      if (eventType === 'm.room.encryption' && stateKey === '') {
        return encrypted ? encryptionEvent : null;
      }
      return eventType === 'm.room.topic' && stateKey === '' ? topicEvent : null;
    },
    maySendMessage: (userId: string) => speakers.includes(userId),
    roomId: '!public:agent-room.test',
  } as unknown as RoomState;
}

function matrixRoom(
  state: RoomState,
  membership = 'join',
  members = ['@z:agent-room.test', '@a:agent-room.test'],
): Room {
  const timeline = {
    getState: () => state,
  } as unknown as EventTimeline;
  return {
    getMyMembership: () => membership,
    getJoinedMembers: () =>
      members.map((userId) => ({ rawDisplayName: `Name of ${userId}`, userId })) as RoomMember[],
    getLiveTimeline: () => timeline,
    name: '  公开大厅  ',
  } as unknown as Room;
}

function matrixClient(
  room: Room | null,
  self: string | null = SELF,
): {
  readonly value: MatrixClient;
} {
  return {
    value: {
      getRoom: () => room,
      getUserId: () => self,
    } as unknown as MatrixClient,
  };
}

/** 谁都当成在线；`notify` 模拟问到了新的在线状态。 */
function presenceSource() {
  const listeners = new Set<() => void>();
  const source = {
    observe: vi.fn((userIds: readonly string[]) => {
      return new Map(userIds.map((userId) => [userId, { state: 'online' } as const]));
    }),
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    notify: () => {
      for (const listener of listeners) listener();
    },
  } satisfies AgentPresenceSource & { notify(): void };
  return source;
}
