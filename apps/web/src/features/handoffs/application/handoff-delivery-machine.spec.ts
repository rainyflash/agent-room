import { createActor, waitFor } from 'xstate';
import { describe, expect, it, vi } from 'vitest';

import { createHandoffDeliveryMachine } from './handoff-delivery-machine';
import type { HandoffApprovalRequest, HandoffGateway } from '@/features/handoffs/domain/handoff';
import {
  acceptedHandoffFixture,
  handoffSnapshotFixture,
  handoffTargetFixture,
} from '@/features/handoffs/testing/handoff-fixtures';
import { err, ok } from '@/shared/result';

const target = handoffTargetFixture;
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
      kind: 'agent',
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

  it('云端提交失败只用同一幂等请求重试，不创建第二个 Handoff', async () => {
    const runtime = gateway();
    runtime.approve.mockResolvedValueOnce(
      err({ code: 'handoff.cloud_unavailable', retryable: true }),
    );
    const actor = start(runtime.value);
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));
    actor.send({ request, type: 'SUBMIT' });
    const failed = await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    expect(failed.can({ type: 'RETRY' })).toBe(true);
    actor.send({ type: 'RETRY' });
    await waitFor(actor, (snapshot) => snapshot.matches('active'));

    expect(runtime.approve).toHaveBeenCalledTimes(2);
    expect(runtime.approve).toHaveBeenNthCalledWith(1, request);
    expect(runtime.approve).toHaveBeenNthCalledWith(2, request);
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
      err({ code: 'handoff.targets_unavailable', retryable: false }),
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

function gateway() {
  const delivered = handoffSnapshotFixture(request.handoffId, 'delivered', request.expiresAtUnixMs);
  const revoked = handoffSnapshotFixture(request.handoffId, 'revoked', request.expiresAtUnixMs);
  const approve = vi.fn<HandoffGateway['approve']>((value) =>
    Promise.resolve(ok(acceptedHandoffFixture(value))),
  );
  const listTargets = vi.fn<HandoffGateway['listTargets']>(() => Promise.resolve(ok([target])));
  const reconcile = vi.fn<HandoffGateway['reconcile']>(() => Promise.resolve(ok(delivered)));
  const revoke = vi.fn<HandoffGateway['revoke']>(() => Promise.resolve(ok(revoked)));
  const value: HandoffGateway = { approve, listTargets, reconcile, revoke };
  return { approve, listTargets, reconcile, revoke, value };
}
