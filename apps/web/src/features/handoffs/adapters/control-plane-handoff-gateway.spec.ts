import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneHandoffGateway } from './control-plane-handoff-gateway';
import type { HandoffApprovalRequest } from '@/features/handoffs/domain/handoff';

const ids = {
  agent: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  content: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  device: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  handoff: '0198b601-77a1-7bb8-83eb-a8fe68c97e49',
  instance: '0198b601-77a1-7bb8-83eb-a8fe68c97e45',
  message: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
  principal: '0198b601-77a1-7bb8-83eb-a8fe68c97e40',
  sourceAgent: '0198b601-77a1-7bb8-83eb-a8fe68c97e41',
  sourceInstance: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
} as const;

describe('ControlPlaneHandoffGateway', () => {
  it('从账号级目录读取在线与离线实例并保留设备归属', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      Response.json({
        targets: [
          {
            adapterType: 'codex-desktop',
            agentAvatarContentId: null,
            agentDisplayName: 'Build Agent',
            agentId: ids.agent,
            agentInstanceId: ids.instance,
            capabilityVersion: '1',
            device: { deviceId: ids.device, label: 'Studio PC', platform: 'windows' },
            instanceStatus: 'offline',
            lastSeenAtUnixMs: 1_700_000_000_000,
            leaseExpiresAtUnixMs: null,
            online: false,
          },
        ],
      }),
    );

    const result = await gateway(fetch).listTargets('!builders:agent-room.test');

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toHaveLength(1);
    expect(result.value[0]?.agentDisplayName).toBe('Build Agent');
    expect(result.value[0]?.device.label).toBe('Studio PC');
    expect(result.value[0]?.instanceId).toBe(ids.instance);
    expect(result.value[0]?.online).toBe(false);
    const [endpoint, init] = fetch.mock.calls[0] ?? [];
    expect(endpoint).toEqual(
      new URL(
        'https://control.agent-room.test/handoff-targets?roomId=%21builders%3Aagent-room.test',
      ),
    );
    expect(init?.cache).toBe('no-store');
    expect(init?.credentials).toBe('include');
    expect(init?.method).toBe('GET');
  });

  it('使用 Handoff ID 幂等创建云端队列并校验完整回显', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ ...handoffResponse(), created: true }, { status: 201 }));

    const result = await gateway(fetch).approve(approvalRequest());

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.kind).toBe('accepted');
    expect(result.value.reused).toBe(false);
    expect(result.value.snapshot.handoffId).toBe(ids.handoff);
    expect(result.value.snapshot.status).toBe('queued');
    expect(result.value.snapshot.targetInstanceId).toBe(ids.instance);
    const [, init] = fetch.mock.calls[0] ?? [];
    expect(init?.credentials).toBe('include');
    expect(new Headers(init?.headers).get('Idempotency-Key')).toBe(ids.handoff);
    expect(init?.method).toBe('POST');
    expect(init?.body).toBe(
      JSON.stringify({
        contentId: ids.content,
        expiresAtUnixMs: 1_700_000_900_000,
        permissions: ['read_text', 'include_metadata'],
        purpose: 'summarize',
        sourceEventId: '$source:agent-room.test',
        sourceMessageId: ids.message,
        sourceRoomId: '!builders:agent-room.test',
        targetInstanceId: ids.instance,
      }),
    );
  });

  it('查询与撤销都只使用原 Handoff ID 并读取云端状态', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json(handoffResponse()))
      .mockResolvedValueOnce(
        Response.json({
          ...handoffResponse(),
          failureCode: null,
          resolvedAtUnixMs: 1_700_000_020_000,
          status: 'revoked',
          version: 1,
        }),
      );
    const client = gateway(fetch);

    expect((await client.reconcile(ids.handoff)).ok).toBe(true);
    const revoked = await client.revoke(ids.handoff);

    expect(revoked.ok).toBe(true);
    if (!revoked.ok) {
      return;
    }
    expect(revoked.value.status).toBe('revoked');
    expect(revoked.value.version).toBe(1);
    expect(fetch.mock.calls.map(([, init]) => init?.method)).toEqual(['GET', 'DELETE']);
  });

  it('拒绝损坏响应并保留云端关联标识与重试语义', async () => {
    const malformedFetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ targets: [{ online: true }] }));
    expect(await gateway(malformedFetch).listTargets('!builders:agent-room.test')).toEqual({
      error: { code: 'handoff.invalid_response', retryable: false },
      ok: false,
    });

    const unavailableFetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'targeted_handoff.dependency_unavailable', retryable: true },
          { headers: { 'x-correlation-id': 'correlation-01' }, status: 503 },
        ),
      );
    expect(await gateway(unavailableFetch).reconcile(ids.handoff)).toEqual({
      error: {
        code: 'handoff.cloud_unavailable',
        correlationId: 'correlation-01',
        retryable: true,
      },
      ok: false,
    });
  });
});

function gateway(fetch: typeof globalThis.fetch) {
  return new ControlPlaneHandoffGateway({
    baseUrl: 'https://control.agent-room.test',
    fetch,
  });
}

function approvalRequest(): HandoffApprovalRequest {
  return {
    expiresAtUnixMs: 1_700_000_900_000,
    handoffId: ids.handoff,
    permissions: ['read_text', 'include_metadata'],
    purpose: 'summarize',
    source: {
      actor: {
        agentId: ids.sourceAgent,
        displayName: 'Source Agent',
        instanceId: ids.sourceInstance,
        kind: 'agent',
        matrixUserId: '@source:agent-room.test',
        provenance: 'autonomous_agent',
      },
      content: {
        contentId: ids.content,
        digestSha256: 'ab'.repeat(32),
        mediaType: 'text/plain',
        sizeBytes: 128,
      },
      matrixEventId: '$source:agent-room.test',
      messageId: ids.message,
      riskFlags: [],
      roomId: '!builders:agent-room.test',
    },
    target: {
      adapterType: 'codex-desktop',
      agentAvatarContentId: null,
      agentDisplayName: 'Build Agent',
      agentId: ids.agent,
      capabilityVersion: '1',
      device: { deviceId: ids.device, label: 'Studio PC', platform: 'windows' },
      instanceId: ids.instance,
      instanceStatus: 'offline',
      lastSeenAtUnixMs: 1_700_000_000_000,
      leaseExpiresAtUnixMs: null,
      online: false,
    },
  };
}

function handoffResponse() {
  return {
    consumedAtUnixMs: null,
    content: {
      byteLength: 128,
      contentId: ids.content,
      mediaType: 'text/plain',
      sha256: 'ab'.repeat(32),
    },
    createdAtUnixMs: 1_700_000_000_000,
    deliveredAtUnixMs: null,
    expiresAtUnixMs: 1_700_000_900_000,
    failureCode: null,
    handoffId: ids.handoff,
    permissions: ['read_text', 'include_metadata'],
    principalId: ids.principal,
    purpose: 'summarize',
    queuedAtUnixMs: 1_700_000_000_000,
    resolvedAtUnixMs: null,
    source: {
      matrixEventId: '$source:agent-room.test',
      matrixRoomId: '!builders:agent-room.test',
      messageId: ids.message,
    },
    status: 'queued',
    target: { agentId: ids.agent, agentInstanceId: ids.instance },
    version: 0,
  };
}
