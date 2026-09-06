import { EventStatus, type MatrixClient, type MatrixEvent, type Room } from 'matrix-js-sdk';
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

  it('私聊只上传密文，且无法读取加密状态时不降级', async () => {
    const base = client(vi.fn());
    const encrypted = new MatrixSdkHumanMessageGateway(
      source({
        getRoom: base.getRoom.bind(base),
        getUserId: base.getUserId.bind(base),
        sendEvent: base.sendEvent.bind(base),
        getStateEvent: () => Promise.resolve({ algorithm: 'm.megolm.v1.aes-sha2' }),
        getDeviceId: () => 'WEB',
        getCrypto: () => readyCrypto(),
      } as unknown as MatrixClient),
    );
    const intent = {
      ...request().event.preview,
      body: '你好',
      roomId: request().roomId,
      submissionId: request().event.id,
      mediaType: 'text/plain' as const,
      sensitivity: 'normal' as const,
    };
    const body = { bytes: new TextEncoder().encode(intent.body), digestSha256: 'a'.repeat(64) };
    const protectedBody = await encrypted.protectBody(intent, body);
    expect(protectedBody.ok).toBe(true);
    if (protectedBody.ok) {
      expect(protectedBody.value.encryption).toBeDefined();
      expect(protectedBody.value.body.bytes).not.toEqual(body.bytes);
    }
    const unavailable = new MatrixSdkHumanMessageGateway(
      source({
        getRoom: base.getRoom.bind(base),
        getUserId: base.getUserId.bind(base),
        sendEvent: base.sendEvent.bind(base),
        getStateEvent: () => Promise.reject(new Error('断网')),
      } as unknown as MatrixClient),
    );
    expect((await unavailable.protectBody(intent, body)).ok).toBe(false);
    await expect(unavailable.publish(request())).resolves.toEqual({
      ok: false,
      error: { kind: 'unavailable', retryable: true },
    });
    expect((await encrypted.publish(request())).ok).toBe(false);
  });

  it('按事务标识只恢复当前用户自己的 v2 事件', () => {
    const matching = event('$accepted', '@rainy:agent-room.test', transactionId);
    const foreign = event('$foreign', '@other:agent-room.test', transactionId);
    const gateway = new MatrixSdkHumanMessageGateway(source(client(vi.fn(), [foreign, matching])));

    expect(gateway.findByTransaction('!public:agent-room.test', transactionId)).toBe('$accepted');
  });

  it('未发送的本地回显不能被查询投递当作服务端确认', () => {
    const unsent = Object.assign(event('$local', '@rainy:agent-room.test', transactionId), {
      status: EventStatus.NOT_SENT,
    });
    const queued = event('~local', '@rainy:agent-room.test', transactionId);
    const gateway = new MatrixSdkHumanMessageGateway(source(client(vi.fn(), [unsent, queued])));
    expect(gateway.findByTransaction('!public:agent-room.test', transactionId)).toBeNull();
  });

  it('加密身份未就绪时在上传前拒绝，重试已上传事务也不会进入发送队列', async () => {
    const sendEvent = vi.fn();
    const base = client(sendEvent);
    const gateway = new MatrixSdkHumanMessageGateway(
      source({
        getRoom: base.getRoom.bind(base),
        getUserId: base.getUserId.bind(base),
        sendEvent: base.sendEvent.bind(base),
        getDeviceId: () => 'WEB',
        getStateEvent: () => Promise.resolve({ algorithm: 'm.megolm.v1.aes-sha2' }),
        getCrypto: () => ({ ...readyCrypto(), isCrossSigningReady: () => Promise.resolve(false) }),
      } as unknown as MatrixClient),
    );
    const intent = {
      ...request().event.preview,
      body: 'Private draft',
      roomId: request().roomId,
      submissionId: request().event.id,
      mediaType: 'text/plain' as const,
      sensitivity: 'normal' as const,
    };
    await expect(
      gateway.protectBody(intent, {
        bytes: new TextEncoder().encode(intent.body),
        digestSha256: 'a'.repeat(64),
      }),
    ).resolves.toEqual({
      ok: false,
      error: { code: 'publication.encryption_not_ready', retryable: true },
    });
    await expect(gateway.publish(encryptedRequest())).resolves.toEqual({
      ok: false,
      error: { kind: 'encryption_not_ready', retryable: true },
    });
    expect(sendEvent).not.toHaveBeenCalled();
  });

  it('明确未加密的失败释放本地队列，而已加密后的网络失败保持未知', async () => {
    for (const encrypted of [false, true]) {
      const local = Object.assign(event('~pending', '@rainy:agent-room.test', transactionId), {
        status: EventStatus.NOT_SENT,
        isEncrypted: () => encrypted,
      });
      const cancelPendingEvent = vi.fn();
      const base = client(vi.fn().mockRejectedValue(new Error('send failed')), [local]);
      const gateway = new MatrixSdkHumanMessageGateway(
        source({
          getRoom: base.getRoom.bind(base),
          getUserId: base.getUserId.bind(base),
          sendEvent: base.sendEvent.bind(base),
          cancelPendingEvent,
          getDeviceId: () => 'WEB',
          getStateEvent: () => Promise.resolve({ algorithm: 'm.megolm.v1.aes-sha2' }),
          getCrypto: () => readyCrypto(),
        } as unknown as MatrixClient),
      );
      await expect(gateway.publish(encryptedRequest())).resolves.toEqual({
        ok: false,
        error: { kind: encrypted ? 'ambiguous' : 'encryption_not_ready', retryable: true },
      });
      expect(cancelPendingEvent).toHaveBeenCalledTimes(encrypted ? 0 : 1);
    }
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
    getMyMembership: () => 'join',
    getLiveTimeline: () => ({ getEvents: () => [...events] }),
  } as unknown as Room;
  return {
    getStateEvent: () =>
      Promise.reject(
        Object.assign(new Error('无加密状态'), { httpStatus: 404, errcode: 'M_NOT_FOUND' }),
      ),
    getRoom: () => room,
    getUserId: () => '@rainy:agent-room.test',
    sendEvent,
  } as unknown as MatrixClient;
}

function event(eventId: string, sender: string, txnId: string): MatrixEvent {
  return {
    status: null,
    getId: () => eventId,
    getSender: () => sender,
    getTxnId: () => txnId,
    getType: () => 'io.github.rainyflash.agentroom.message.preview.v2',
  } as unknown as MatrixEvent;
}

function readyCrypto() {
  return {
    isEncryptionEnabledInRoom: () => Promise.resolve(true),
    isCrossSigningReady: () => Promise.resolve(true),
    getDeviceVerificationStatus: () => Promise.resolve({ crossSigningVerified: true }),
  };
}

function encryptedRequest(): MatrixPublicationRequest {
  const plain = request();
  return {
    ...plain,
    event: {
      ...plain.event,
      content: {
        ...plain.event.content,
        encryption: {
          algorithm: 'io.github.rainyflash.agentroom.content.aes-256-gcm.v1',
          contextId: plain.event.id,
          keyBase64Url: 'a'.repeat(43),
          nonceBase64Url: 'a'.repeat(16),
          plaintextSizeBytes: 5,
        },
      },
    },
  };
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
