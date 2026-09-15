import type { EventTimeline, MatrixClient, MatrixEvent, Room } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import {
  MatrixSdkMessageSource,
  matrixMessagePreviewEventType,
  matrixMessageRevisionEventType,
  matrixModerationNoticeEventType,
} from './matrix-message-source';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

describe('MatrixSdkMessageSource', () => {
  it('expands cached history before requesting another real page', async () => {
    const events = Array.from({ length: 600 }, (_, index) =>
      matrixEvent(matrixMessagePreviewEventType, `$event${String(index)}`, index),
    );
    const room = matrixRoom(
      events,
      [],
      () => 'join',
      () => 'older',
    );
    const client = matrixClient(room);
    const fetch = vi.spyOn(client, 'scrollback');
    const registry = new MatrixClientRegistry();
    registry.replace(client);
    const source = new MatrixSdkMessageSource(registry);
    expect((await source.loadOlder('!public:agent-room.test')).ok).toBe(true);
    expect(fetch).not.toHaveBeenCalled();
    const read = source.read('!public:agent-room.test');
    expect(read.kind === 'ready' && read.room.windowSize).toBe(400);
    await source.loadOlder('!public:agent-room.test');
    await source.loadOlder('!public:agent-room.test');
    expect(fetch).toHaveBeenCalledExactlyOnceWith(room, 200);
  });

  it('coalesces pagination and rejects results after membership or account changes', async () => {
    let membership = 'join';
    const room = matrixRoom(
      [],
      [],
      () => membership,
      () => 'older',
    );
    const client = matrixClient(room);
    let finish: () => void = () => undefined;
    vi.spyOn(client, 'scrollback').mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = () => {
            resolve(room);
          };
        }),
    );
    const registry = new MatrixClientRegistry();
    registry.replace(client);
    const source = new MatrixSdkMessageSource(registry);
    const pending = source.loadOlder('!public:agent-room.test');
    expect(source.loadOlder('!public:agent-room.test')).toBe(pending);
    membership = 'leave';
    finish();
    expect(await pending).toEqual({
      ok: false,
      error: { code: 'history.session_changed', retryable: true },
    });
    expect(source.read('!public:agent-room.test').kind).toBe('room-not-joined');
    membership = 'join';
    const oldAccount = source.loadOlder('!public:agent-room.test');
    registry.replace(matrixClient(matrixRoom([])));
    finish();
    expect((await oldAccount).ok).toBe(false);
    const read = source.read('!public:agent-room.test');
    expect(read.kind === 'ready' && read.room.windowSize).toBe(200);
  });

  it('reports paging failures without growing the search coverage', async () => {
    const client = matrixClient(
      matrixRoom(
        [],
        [],
        () => 'join',
        () => 'older',
      ),
    );
    vi.spyOn(client, 'scrollback').mockRejectedValue(new Error('offline'));
    const registry = new MatrixClientRegistry();
    registry.replace(client);
    const source = new MatrixSdkMessageSource(registry);
    expect((await source.loadOlder('!public:agent-room.test')).ok).toBe(false);
    const read = source.read('!public:agent-room.test');
    expect(read.kind === 'ready' && read.room.windowSize).toBe(200);
  });
  it('只复制当前房间实时时间线中的 Agent Room 消息事件', () => {
    const preview = matrixEvent(matrixMessagePreviewEventType, '$preview', 20);
    const revision = matrixEvent(matrixMessageRevisionEventType, '$revision', 30);
    const ordinary = matrixEvent('m.room.message', '$ordinary', 10);
    const future = matrixEvent('io.github.rainyflash.agentroom.message.future.v9', '$future', 35);
    const legacy = matrixEvent(
      ['org', 'agentroom', 'message', 'preview', 'v1'].join('.'),
      '$legacy',
      36,
    );
    const moderation = matrixEvent(matrixModerationNoticeEventType, '$moderation', 40);
    const registry = new MatrixClientRegistry();
    registry.replace(
      matrixClient(matrixRoom([ordinary, preview, revision, future, legacy], [moderation])),
    );

    const source = new MatrixSdkMessageSource(registry);

    expect(source.read('!public:agent-room.test')).toEqual({
      kind: 'ready',
      room: {
        windowSize: 200,
        hasOlder: false,
        roomId: '!public:agent-room.test',
        timelineEvents: [
          {
            content: { eventType: matrixMessagePreviewEventType },
            endToEndEncrypted: false,
            eventId: '$preview',
            sender: '@agent:agent-room.test',
            serverTimestamp: 20,
            type: matrixMessagePreviewEventType,
          },
          {
            content: { eventType: matrixMessageRevisionEventType },
            endToEndEncrypted: false,
            eventId: '$revision',
            sender: '@agent:agent-room.test',
            serverTimestamp: 30,
            type: matrixMessageRevisionEventType,
          },
          {
            content: { eventType: 'io.github.rainyflash.agentroom.message.future.v9' },
            endToEndEncrypted: false,
            eventId: '$future',
            sender: '@agent:agent-room.test',
            serverTimestamp: 35,
            type: 'io.github.rainyflash.agentroom.message.future.v9',
          },
          {
            content: { eventType: ['org', 'agentroom', 'message', 'preview', 'v1'].join('.') },
            endToEndEncrypted: false,
            eventId: '$legacy',
            sender: '@agent:agent-room.test',
            serverTimestamp: 36,
            type: ['org', 'agentroom', 'message', 'preview', 'v1'].join('.'),
          },
          {
            content: { eventType: matrixModerationNoticeEventType },
            endToEndEncrypted: false,
            eventId: '$moderation',
            sender: '@agent:agent-room.test',
            serverTimestamp: 40,
            type: matrixModerationNoticeEventType,
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
    isEncrypted: () => false,
    getSender: () => '@agent:agent-room.test',
    getTs: () => serverTimestamp,
    getType: () => type,
  } as unknown as MatrixEvent;
}

function matrixRoom(
  events: readonly MatrixEvent[],
  stateEvents: readonly MatrixEvent[] = [],
  membership: () => string = () => 'join',
  pagination: () => string | null = () => null,
): Room {
  return {
    getMyMembership: membership,
    getLiveTimeline: () =>
      ({
        getPaginationToken: pagination,
        getEvents: () => [...events],
        getState: () => ({
          getStateEvents: () => [...stateEvents],
        }),
      }) as unknown as EventTimeline,
  } as unknown as Room;
}

function matrixClient(room: Room | null): MatrixClient {
  return {
    getRoom: () => room,
    scrollback: () => Promise.resolve(room),
  } as unknown as MatrixClient;
}
