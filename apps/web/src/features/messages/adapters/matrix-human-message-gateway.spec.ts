import type { MatrixClient, MatrixEvent, Room } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkHumanMessageGateway } from './matrix-human-message-gateway';
import type { MatrixPublicationRequest } from '@/features/messages/domain/publication';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

const transactionId = 'agent-room-message-01990d9e-8400-7000-8000-000000000003';

describe('MatrixSdkHumanMessageGateway', () => {
  it('使用当前 Matrix 用户和稳定事务发送 v2 Human 事件', async () => {
    const sendEvent = vi.fn().mockResolvedValue({ event_id: '$accepted' });
    const gateway = new MatrixSdkHumanMessageGateway(source(client(sendEvent)));

    const result = await gateway.publish(request());

    expect(result).toEqual({ ok: true, value: { matrixEventId: '$accepted' } });
    expect(gateway.currentUserId()).toBe('@rainy:agent-room.test');
    expect(sendEvent).toHaveBeenCalledWith(
      '!public:agent-room.test',
      'io.github.rainyflash.agentroom.message.preview.v2',
      request().event,
      transactionId,
    );
  });

  it('按事务标识只恢复当前用户自己的 v2 事件', () => {
    const matching = event('$accepted', '@rainy:agent-room.test', transactionId);
    const foreign = event('$foreign', '@other:agent-room.test', transactionId);
    const gateway = new MatrixSdkHumanMessageGateway(source(client(vi.fn(), [foreign, matching])));

    expect(gateway.findByTransaction('!public:agent-room.test', transactionId)).toBe('$accepted');
  });

  it('区分明确 4xx 拒绝、未知提交和本地不可用', async () => {
    const rejectedError = Object.assign(new Error('forbidden'), { httpStatus: 403 });
    const rejected = new MatrixSdkHumanMessageGateway(
      source(client(vi.fn().mockRejectedValue(rejectedError))),
    );
    const ambiguous = new MatrixSdkHumanMessageGateway(
      source(client(vi.fn().mockRejectedValue(new TypeError('offline')))),
    );
    const unavailable = new MatrixSdkHumanMessageGateway(source(null));

    await expect(rejected.publish(request())).resolves.toEqual({
      error: { kind: 'rejected', retryable: false },
      ok: false,
    });
    await expect(ambiguous.publish(request())).resolves.toEqual({
      error: { kind: 'ambiguous', retryable: true },
      ok: false,
    });
    await expect(unavailable.publish(request())).resolves.toEqual({
      error: { kind: 'unavailable', retryable: true },
      ok: false,
    });
  });
});

function source(value: MatrixClient | null): MatrixClientSource {
  return { current: () => value, subscribe: () => noop };
}

function client(sendEvent: ReturnType<typeof vi.fn>, events: readonly MatrixEvent[] = []) {
  const room = {
    getLiveTimeline: () => ({ getEvents: () => [...events] }),
  } as unknown as Room;
  return {
    getRoom: () => room,
    getUserId: () => '@rainy:agent-room.test',
    sendEvent,
  } as unknown as MatrixClient;
}

function event(eventId: string, sender: string, txnId: string): MatrixEvent {
  return {
    getId: () => eventId,
    getSender: () => sender,
    getTxnId: () => txnId,
    getType: () => 'io.github.rainyflash.agentroom.message.preview.v2',
  } as unknown as MatrixEvent;
}

function request(): MatrixPublicationRequest {
  const content = {
    contentId: '01990d9e-8400-7000-8000-000000000004',
    digestSha256: 'a'.repeat(64),
    fetchMode: 'on_demand',
    mediaType: 'text/markdown',
    sizeBytes: 5,
  } as const;
  return {
    event: {
      actor: {
        displayName: 'Rainy',
        kind: 'human',
        matrixUserId: '@rainy:agent-room.test',
        principalId: '01990d9e-8400-7000-8000-000000000001',
      },
      content,
      correlationId: '01990d9e-8400-7000-8000-000000000003',
      createdAt: '2026-08-30T12:00:00.000Z',
      eventType: 'io.github.rainyflash.agentroom.message.preview.v2',
      id: '01990d9e-8400-7000-8000-000000000003',
      preview: {
        contentType: 'text/markdown',
        riskFlags: [],
        sensitivity: 'normal',
        summary: '摘要',
        title: '标题',
      },
      roomId: '!public:agent-room.test',
      schemaVersion: '2.0',
    },
    roomId: '!public:agent-room.test',
    transactionId,
  };
}

function noop(): void {
  return undefined;
}
