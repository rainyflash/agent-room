import type { EventTimeline, MatrixClient, MatrixEvent, Room } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import {
  MatrixSdkMessageSource,
  matrixMessagePreviewEventType,
  matrixMessageRevisionEventType,
} from './matrix-message-source';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

describe('MatrixSdkMessageSource', () => {
  it('只复制当前房间实时时间线中的 Agent Room 消息事件', () => {
    const preview = matrixEvent(matrixMessagePreviewEventType, '$preview', 20);
    const revision = matrixEvent(matrixMessageRevisionEventType, '$revision', 30);
    const ordinary = matrixEvent('m.room.message', '$ordinary', 10);
    const registry = new MatrixClientRegistry();
    registry.replace(matrixClient(matrixRoom([ordinary, preview, revision])));

    const source = new MatrixSdkMessageSource(registry);

    expect(source.read('!public:agent-room.test')).toEqual({
      kind: 'ready',
      room: {
        roomId: '!public:agent-room.test',
        timelineEvents: [
          {
            content: { eventType: matrixMessagePreviewEventType },
            eventId: '$preview',
            sender: '@agent:agent-room.test',
            serverTimestamp: 20,
            type: matrixMessagePreviewEventType,
          },
          {
            content: { eventType: matrixMessageRevisionEventType },
            eventId: '$revision',
            sender: '@agent:agent-room.test',
            serverTimestamp: 30,
            type: matrixMessageRevisionEventType,
          },
        ],
      },
    });
  });

  it('没有 Matrix 客户端或房间时返回明确边界', () => {
    const registry = new MatrixClientRegistry();
    const source = new MatrixSdkMessageSource(registry);

    expect(source.read('!public:agent-room.test')).toEqual({ kind: 'matrix-unavailable' });

    registry.replace(matrixClient(null));
    expect(source.read('!public:agent-room.test')).toEqual({ kind: 'room-not-joined' });
  });

  it('把客户端同步活动转发给订阅者并正确释放', () => {
    const registry = new MatrixClientRegistry();
    const client = matrixClient(matrixRoom([]));
    registry.replace(client);
    const source = new MatrixSdkMessageSource(registry);
    const listener = vi.fn();

    const unsubscribe = source.subscribe('!public:agent-room.test', listener);
    registry.refresh(client);
    unsubscribe();
    registry.refresh(client);

    expect(listener).toHaveBeenCalledOnce();
  });
});

function matrixEvent(type: string, eventId: string, serverTimestamp: number): MatrixEvent {
  return {
    getContent: () => ({ eventType: type }),
    getId: () => eventId,
    getSender: () => '@agent:agent-room.test',
    getTs: () => serverTimestamp,
    getType: () => type,
  } as unknown as MatrixEvent;
}

function matrixRoom(events: readonly MatrixEvent[]): Room {
  return {
    getLiveTimeline: () =>
      ({
        getEvents: () => [...events],
      }) as unknown as EventTimeline,
  } as unknown as Room;
}

function matrixClient(room: Room | null): MatrixClient {
  return {
    getRoom: () => room,
  } as unknown as MatrixClient;
}
