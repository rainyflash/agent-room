import { createActor, waitFor } from 'xstate';
import { describe, expect, it, vi } from 'vitest';

import { createMessagePublicationMachine } from './message-publication-machine';
import type {
  MessagePublicationRequest,
  MessagePublicationResult,
  MessagePublisher,
  MessagePublisherIdentity,
} from '@/features/messages/domain/publication';
import { err, ok } from '@/shared/result';

const identity: MessagePublisherIdentity = {
  displayName: 'Build Agent',
  kind: 'human',
  matrixUserId: '@build-agent:agent-room.test',
  principalId: '01990d9e-8400-7000-8000-000000000001',
  source: 'matrix_human_session',
};
const request: MessagePublicationRequest = {
  body: '# Verified build\n\nReview is requested.',
  language: 'en',
  mediaType: 'text/markdown',
  riskFlags: [],
  roomId: '!room:agent-room.test',
  sensitivity: 'normal',
  submissionId: '01990d9e-8400-7000-8000-000000000003',
  summary: 'The build completed and is ready for review.',
  title: 'Build completed',
};
const published: MessagePublicationResult = ok({
  kind: 'published',
  matrixEventId: '$event',
  reused: false,
  submissionId: request.submissionId,
});
const unknown: MessagePublicationResult = ok({
  kind: 'pending_reconciliation',
  submissionId: request.submissionId,
  transactionId: `agent-room-message-${request.submissionId}`,
});

describe('消息发布状态机', () => {
  it('解析真实身份后按上传和提交阶段进入已接受状态', async () => {
    const runtime = publisher();
    runtime.publish.mockImplementation((_request, onProgress) => {
      onProgress('uploading');
      onProgress('submitting');
      return Promise.resolve(published);
    });
    const actor = open(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({ request, type: 'SUBMIT' });
    const done = await waitFor(actor, (snapshot) => snapshot.matches('published'));

    expect(runtime.resolveIdentity).toHaveBeenCalledOnce();
    expect(runtime.publish).toHaveBeenCalledWith(request, expect.any(Function));
    expect(done.context.identity).toEqual(identity);
    expect(done.context.outcome).toEqual(published.ok ? published.value : null);
    actor.stop();
  });

  it('状态未知时没有重发转换，只允许按原提交标识对账', async () => {
    const runtime = publisher({ publishResults: [unknown], reconcileResults: [published] });
    const actor = open(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({ request, type: 'SUBMIT' });
    const uncertain = await waitFor(actor, (snapshot) => snapshot.matches('unknown'));

    expect(uncertain.can({ type: 'RETRY' })).toBe(false);
    expect(uncertain.can({ request, type: 'SUBMIT' })).toBe(false);
    expect(uncertain.can({ type: 'RECONCILE' })).toBe(true);
    actor.send({ type: 'RETRY' });
    actor.send({ request, type: 'SUBMIT' });
    expect(runtime.publish).toHaveBeenCalledOnce();

    actor.send({ type: 'RECONCILE' });
    await waitFor(actor, (snapshot) => snapshot.matches('published'));

    expect(runtime.reconcile).toHaveBeenCalledOnce();
    expect(runtime.reconcile).toHaveBeenCalledWith(request.submissionId);
    expect(runtime.publish).toHaveBeenCalledOnce();
    actor.stop();
  });

  it('发布适配器异常中断也按未知提交处理，不猜测为安全失败', async () => {
    const runtime = publisher({ publishResults: [], reconcileResults: [published] });
    runtime.publish.mockRejectedValueOnce(new Error('transport interrupted'));
    const actor = open(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({ request, type: 'SUBMIT' });

    const uncertain = await waitFor(actor, (snapshot) => snapshot.matches('unknown'));

    expect(uncertain.context.outcome).toEqual({
      kind: 'pending_reconciliation',
      submissionId: request.submissionId,
      transactionId: `agent-room-message-${request.submissionId}`,
    });
    expect(uncertain.can({ type: 'RETRY' })).toBe(false);
    actor.send({ type: 'RECONCILE' });
    await waitFor(actor, (snapshot) => snapshot.matches('published'));
    expect(runtime.publish).toHaveBeenCalledOnce();
    actor.stop();
  });

  it('明确未提交的可重试失败复用同一个幂等意图', async () => {
    const retryable = err({
      code: 'publication.matrix_rejected' as const,
      retryable: true,
    });
    const runtime = publisher({ publishResults: [retryable, published] });
    const actor = open(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({ request, type: 'SUBMIT' });
    const failed = await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    expect(failed.can({ type: 'RETRY' })).toBe(true);
    actor.send({ type: 'RETRY' });
    await waitFor(actor, (snapshot) => snapshot.matches('published'));

    expect(runtime.publish).toHaveBeenCalledTimes(2);
    expect(runtime.publish.mock.calls[0]?.[0]).toBe(request);
    expect(runtime.publish.mock.calls[1]?.[0]).toBe(request);
    actor.stop();
  });

  it('对账失败后的重试仍然只做查询，不重新发布', async () => {
    const retryable = err({
      code: 'publication.matrix_rejected' as const,
      retryable: true,
    });
    const runtime = publisher({
      publishResults: [unknown],
      reconcileResults: [retryable, published],
    });
    const actor = open(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({ request, type: 'SUBMIT' });
    await waitFor(actor, (snapshot) => snapshot.matches('unknown'));
    actor.send({ type: 'RECONCILE' });
    await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    actor.send({ type: 'RETRY' });
    await waitFor(actor, (snapshot) => snapshot.matches('published'));

    expect(runtime.publish).toHaveBeenCalledOnce();
    expect(runtime.reconcile).toHaveBeenCalledTimes(2);
    actor.stop();
  });

  it('非法房间或非 UUIDv7 提交不会抵达发布端口', async () => {
    const runtime = publisher();
    const actor = open(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({
      request: { ...request, roomId: 'bad-room', submissionId: crypto.randomUUID() },
      type: 'SUBMIT',
    });
    const failed = await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    expect(failed.context.failure?.code).toBe('publication.invalid_intent');
    expect(runtime.publish).not.toHaveBeenCalled();
    actor.stop();
  });
});

function open(value: MessagePublisher) {
  const actor = createActor(createMessagePublicationMachine(value)).start();
  actor.send({ roomId: request.roomId, type: 'OPEN' });
  return actor;
}

function publisher(
  options: {
    readonly publishResults?: readonly MessagePublicationResult[];
    readonly reconcileResults?: readonly MessagePublicationResult[];
  } = {},
) {
  const publish = vi.fn<MessagePublisher['publish']>();
  for (const result of options.publishResults ?? [published]) {
    publish.mockResolvedValueOnce(result);
  }
  const reconcile = vi.fn<MessagePublisher['reconcile']>();
  for (const result of options.reconcileResults ?? [unknown]) {
    reconcile.mockResolvedValueOnce(result);
  }
  const resolveIdentity = vi.fn<MessagePublisher['resolveIdentity']>(() =>
    Promise.resolve(ok(identity)),
  );
  const value: MessagePublisher = { publish, reconcile, resolveIdentity };
  return { publish, reconcile, resolveIdentity, value };
}
