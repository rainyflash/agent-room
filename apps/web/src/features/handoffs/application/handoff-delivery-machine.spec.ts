import { createActor, waitFor } from 'xstate';
import { describe, expect, it, vi } from 'vitest';

import { createHandoffDeliveryMachine } from './handoff-delivery-machine';
import type {
  HandoffApprovalRequest,
  HandoffGateway,
  HandoffSnapshot,
  HandoffSubmissionOutcome,
  HandoffTarget,
} from '@/features/handoffs/domain/handoff';
import { err, ok } from '@/shared/result';

const target: HandoffTarget = {
  agentId: '01990d9e-8400-7000-8000-000000000005',
  displayName: 'Local Codex',
  instanceId: '01990d9e-8400-7000-8000-000000000006',
};
const request: HandoffApprovalRequest = {
  expiresAtUnixMs: 901_000,
  handoffId: '01990d9e-8400-7000-8000-000000000010',
  permissions: ['read_text', 'include_metadata'],
  purpose: 'summarize',
  source: {
    actor: {
      agentId: '01990d9e-8400-7000-8000-000000000001',
      displayName: 'Remote Agent',
      instanceId: '01990d9e-8400-7000-8000-000000000002',
      matrixUserId: '@remote:agent-room.test',
      provenance: 'autonomous_agent',
    },
    content: {
      contentId: '01990d9e-8400-7000-8000-000000000003',
      digestSha256: 'ab'.repeat(32),
      mediaType: 'text/plain',
      sizeBytes: 128,
    },
    matrixEventId: '$source:agent-room.test',
    messageId: '01990d9e-8400-7000-8000-000000000004',
    riskFlags: ['untrusted_instructions'],
    roomId: '!builders:agent-room.test',
  },
  target,
};

describe('上下文交付状态机', () => {
  it('只在目标解析完成且意图有效后批准并允许撤销', async () => {
    const runtime = gateway();
    const actor = start(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({ request, type: 'SUBMIT' });
    await waitFor(actor, (snapshot) => snapshot.matches('active'));
    actor.send({ type: 'REVOKE' });
    const revoked = await waitFor(actor, (snapshot) => snapshot.matches('resolved'));

    expect(runtime.approve).toHaveBeenCalledWith(request);
    expect(runtime.revoke).toHaveBeenCalledWith(request.handoffId);
    expect(revoked.context.snapshot?.status).toBe('revoked');
    actor.stop();
  });

  it('未知提交不能重发，只允许按原 Handoff ID 查询', async () => {
    const runtime = gateway({
      approveResult: ok({ handoffId: request.handoffId, kind: 'delivery_uncertain' }),
    });
    const actor = start(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({ request, type: 'SUBMIT' });
    const uncertain = await waitFor(actor, (snapshot) => snapshot.matches('uncertain'));

    expect(uncertain.can({ request, type: 'SUBMIT' })).toBe(false);
    expect(uncertain.can({ type: 'QUERY' })).toBe(true);
    actor.send({ type: 'QUERY' });
    await waitFor(actor, (snapshot) => snapshot.matches('active'));

    expect(runtime.approve).toHaveBeenCalledOnce();
    expect(runtime.reconcile).toHaveBeenCalledWith(request.handoffId);
    actor.stop();
  });

  it('非法范围在调用端口前失败关闭', async () => {
    const runtime = gateway();
    const actor = start(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({ request: { ...request, permissions: [] }, type: 'SUBMIT' });
    const failed = await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    expect(failed.context.failure?.code).toBe('handoff.invalid_intent');
    expect(runtime.approve).not.toHaveBeenCalled();
    actor.stop();
  });

  it('目标解析失败不会显示伪造实例', async () => {
    const runtime = gateway();
    runtime.listTargets.mockResolvedValueOnce(
      err({ code: 'handoff.bridge_unavailable', retryable: false }),
    );
    const actor = start(runtime.value);

    const failed = await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    expect(failed.context.targets).toEqual([]);
    expect(runtime.approve).not.toHaveBeenCalled();
    actor.stop();
  });
});

function start(value: HandoffGateway) {
  return createActor(
    createHandoffDeliveryMachine({
      gateway: value,
      now: () => 1_000,
      roomId: request.source.roomId,
    }),
  ).start();
}

function gateway(
  options: {
    readonly approveResult?: ReturnType<typeof ok<HandoffSubmissionOutcome>>;
  } = {},
) {
  const approved =
    options.approveResult ??
    ok({ handoffId: request.handoffId, kind: 'submitted' as const, reused: false });
  const delivered: HandoffSnapshot = {
    expiresAtUnixMs: request.expiresAtUnixMs,
    handoffId: request.handoffId,
    status: 'delivered',
  };
  const revoked: HandoffSnapshot = { ...delivered, status: 'revoked' };
  const approve = vi.fn<HandoffGateway['approve']>(() => Promise.resolve(approved));
  const listTargets = vi.fn<HandoffGateway['listTargets']>(() => Promise.resolve(ok([target])));
  const reconcile = vi.fn<HandoffGateway['reconcile']>(() => Promise.resolve(ok(delivered)));
  const revoke = vi.fn<HandoffGateway['revoke']>(() => Promise.resolve(ok(revoked)));
  const value: HandoffGateway = { approve, listTargets, reconcile, revoke };
  return { approve, listTargets, reconcile, revoke, value };
}
