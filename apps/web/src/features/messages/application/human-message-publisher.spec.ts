import { describe, expect, it, vi } from 'vitest';

import { HumanMessagePublisher } from './human-message-publisher';
import type {
  HumanMatrixPublicationGateway,
  MessageBodyPreparer,
  MessagePublicationContentGateway,
  MessagePublicationRequest,
  MessageSubmissionJournal,
  MessageSubmissionRecord,
} from '@/features/messages/domain/publication';
import type { ControlPlaneGateway, WebSession } from '@/features/session/domain/session';
import { err, ok } from '@/shared/result';

const submissionId = '01990d9e-8400-7000-8000-000000000003';
const contentId = '01990d9e-8400-7000-8000-000000000004';
const roomId = '!public:agent-room.test';

describe('HumanMessagePublisher', () => {
  it('直接使用 Agent Room 用户会话发布无实例签名的 Human v2 消息', async () => {
    const runtime = dependencies();
    const publisher = new HumanMessagePublisher(runtime.value);
    const progress = vi.fn();

    const result = await publisher.publish(request(), progress);

    expect(result).toEqual({
      ok: true,
      value: { kind: 'published', matrixEventId: '$accepted', reused: false, submissionId },
    });
    expect(progress).toHaveBeenNthCalledWith(1, 'uploading');
    expect(progress).toHaveBeenNthCalledWith(2, 'submitting');
    expect(progress).toHaveBeenNthCalledWith(3, 'binding');
    expect(runtime.matrix.publish).toHaveBeenCalledOnce();
    const event = runtime.matrix.publish.mock.calls[0]?.[0].event;
    expect(event?.actor).toEqual({
      displayName: 'Rainy',
      kind: 'human',
      matrixUserId: '@rainy:agent-room.test',
      principalId: '01990d9e-8400-7000-8000-000000000001',
    });
    expect(event).not.toHaveProperty('signature');
    expect(JSON.stringify(event)).not.toContain(request().body);
    expect(runtime.content.upload).toHaveBeenCalledWith(
      expect.objectContaining({ roomId, submissionId }),
    );
    expect(runtime.content.bind).toHaveBeenCalledWith({
      contentId,
      matrixEventId: '$accepted',
      roomId,
    });
  });

  it('拒绝 Control Plane 身份与 Matrix 用户不一致的会话', async () => {
    const runtime = dependencies({ matrixUserId: '@other:agent-room.test' });
    const publisher = new HumanMessagePublisher(runtime.value);

    await expect(publisher.resolveIdentity()).resolves.toEqual({
      error: { code: 'publication.identity_unavailable', retryable: true },
      ok: false,
    });
    await expect(publisher.publish(request(), vi.fn())).resolves.toEqual({
      error: { code: 'publication.identity_unavailable', retryable: true },
      ok: false,
    });
    expect(runtime.content.upload).not.toHaveBeenCalled();
    expect(runtime.matrix.publish).not.toHaveBeenCalled();
  });

  it('远端提交未知时只按相同 Matrix 事务恢复，不生成第二个意图', async () => {
    const runtime = dependencies({
      matrixResults: [
        err({ kind: 'ambiguous' as const, retryable: true }),
        ok({ matrixEventId: '$recovered' }),
      ],
    });
    const publisher = new HumanMessagePublisher(runtime.value);

    const uncertain = await publisher.publish(request(), vi.fn());
    const recovered = await publisher.reconcile(submissionId);

    expect(uncertain).toEqual({
      ok: true,
      value: {
        kind: 'pending_reconciliation',
        submissionId,
        transactionId: `agent-room-message-${submissionId}`,
      },
    });
    expect(recovered).toEqual({
      ok: true,
      value: { kind: 'published', matrixEventId: '$recovered', reused: true, submissionId },
    });
    expect(runtime.content.upload).toHaveBeenCalledOnce();
    expect(runtime.matrix.publish).toHaveBeenCalledTimes(2);
    expect(runtime.matrix.publish.mock.calls[0]?.[0].transactionId).toBe(
      runtime.matrix.publish.mock.calls[1]?.[0].transactionId,
    );
    expect(runtime.matrix.publish.mock.calls[0]?.[0].event).toEqual(
      runtime.matrix.publish.mock.calls[1]?.[0].event,
    );
  });

  it('Matrix 已接受但绑定失败时只重试绑定', async () => {
    const runtime = dependencies({
      bindResults: [
        err({ code: 'publication.content_rejected' as const, retryable: true }),
        ok(undefined),
      ],
    });
    const publisher = new HumanMessagePublisher(runtime.value);

    const pending = await publisher.publish(request(), vi.fn());
    const reconciled = await publisher.reconcile(submissionId);

    expect(pending).toEqual({
      ok: true,
      value: { kind: 'accepted_binding_pending', matrixEventId: '$accepted', submissionId },
    });
    expect(reconciled).toEqual({
      ok: true,
      value: { kind: 'published', matrixEventId: '$accepted', reused: true, submissionId },
    });
    expect(runtime.matrix.publish).toHaveBeenCalledOnce();
    expect(runtime.content.bind).toHaveBeenCalledTimes(2);
  });

  it('同一 submissionId 绑定不同意图时失败关闭', async () => {
    const runtime = dependencies();
    const publisher = new HumanMessagePublisher(runtime.value);
    await publisher.publish(request(), vi.fn());

    const result = await publisher.publish({ ...request(), title: '另一条消息' }, vi.fn());

    expect(result).toEqual({
      error: { code: 'publication.invalid_intent', retryable: false },
      ok: false,
    });
    expect(runtime.content.upload).toHaveBeenCalledOnce();
    expect(runtime.matrix.publish).toHaveBeenCalledOnce();
  });
});

function dependencies(
  options: {
    readonly bindResults?: readonly Awaited<ReturnType<MessagePublicationContentGateway['bind']>>[];
    readonly matrixResults?: readonly Awaited<
      ReturnType<HumanMatrixPublicationGateway['publish']>
    >[];
    readonly matrixUserId?: string;
  } = {},
) {
  const bodyPreparer: MessageBodyPreparer = {
    prepare: (body) => {
      const encoded = new TextEncoder().encode(body);
      const bytes = new Uint8Array(encoded.byteLength);
      bytes.set(encoded);
      return Promise.resolve(ok({ bytes, digestSha256: digest(body) }));
    },
  };
  const upload = vi.fn<MessagePublicationContentGateway['upload']>((uploadRequest) =>
    Promise.resolve(
      ok({
        contentId,
        digestSha256: uploadRequest.body.digestSha256,
        mediaType: uploadRequest.mediaType,
        sizeBytes: uploadRequest.body.bytes.byteLength,
      }),
    ),
  );
  const bind = vi.fn<MessagePublicationContentGateway['bind']>();
  for (const result of options.bindResults ?? [ok(undefined)]) {
    bind.mockResolvedValueOnce(result);
  }
  const content: MessagePublicationContentGateway = { bind, upload };
  const journal = new MemoryJournal();
  const publish = vi.fn<HumanMatrixPublicationGateway['publish']>();
  for (const result of options.matrixResults ?? [ok({ matrixEventId: '$accepted' })]) {
    publish.mockResolvedValueOnce(result);
  }
  const matrix = {
    protectBody: (_request, body) => Promise.resolve(ok({ body })),
    currentUserId: () => options.matrixUserId ?? '@rainy:agent-room.test',
    findByTransaction: vi.fn(() => null),
    publish,
  } satisfies HumanMatrixPublicationGateway;
  const session = {
    readSession: () => Promise.resolve(ok(webSession())),
  } satisfies Pick<ControlPlaneGateway, 'readSession'>;
  return {
    content: { bind, upload },
    matrix: { ...matrix, publish },
    value: { bodyPreparer, clock: () => 1_777_550_400_000, content, journal, matrix, session },
  };
}

class MemoryJournal implements MessageSubmissionJournal {
  releaseBody() {
    return undefined;
  }
  readBody() {
    return ok(null);
  }
  writeBody() {
    return ok(undefined);
  }
  readonly #records = new Map<string, MessageSubmissionRecord>();

  read(submissionId: string) {
    return ok(this.#records.get(submissionId) ?? null);
  }

  write(record: MessageSubmissionRecord) {
    this.#records.set(record.submissionId, record);
    return ok(undefined);
  }
}

function webSession(): WebSession {
  return {
    authenticatedAtUnixMs: 1,
    displayName: 'Rainy',
    expiresAtUnixMs: 9_999_999_999_999,
    locale: 'zh-CN',
    matrixUserId: '@rainy:agent-room.test',
    principalId: '01990d9e-8400-7000-8000-000000000001',
    recentlyAuthenticated: false,
  };
}

function request(): MessagePublicationRequest {
  return {
    body: '# 私密正文\n\nsecret body',
    language: 'zh-CN',
    mediaType: 'text/markdown',
    riskFlags: [],
    roomId,
    sensitivity: 'normal',
    submissionId,
    summary: '是否打开由接收者决定。',
    title: '用户消息',
  };
}

function digest(value: string): string {
  let hash = 0;
  for (const byte of new TextEncoder().encode(value)) {
    hash = (hash * 31 + byte) >>> 0;
  }
  return hash.toString(16).padStart(8, '0').repeat(8);
}

it('聊天完整文本和引用写入 Matrix，切换账号后禁止恢复旧账号提交', async () => {
  const runtime = dependencies();
  const publisher = new HumanMessagePublisher(runtime.value);
  const intent = {
    ...request(),
    mediaType: 'text/plain' as const,
    body: '请一起讨论',
    conversation: { text: '请一起讨论', mentions: ['@agent:matrix.test'] },
    relation: { kind: 'reply' as const, targetMessageId: '01990d9e-8400-7000-8000-000000000099' },
  };
  const result = await publisher.publish(intent, () => undefined);
  expect(result.ok).toBe(true);
  const sent = runtime.matrix.publish.mock.calls[0]?.[0];
  expect(sent?.event.preview.conversation).toEqual(intent.conversation);
  expect(sent?.event.relation).toEqual(intent.relation);
  runtime.value.matrix.currentUserId = () => '@other:agent-room.test';
  expect((await publisher.reconcile(submissionId)).ok).toBe(false);
  expect(runtime.matrix.publish).toHaveBeenCalledOnce();
});
