import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneModerationClient } from './control-plane-moderation-client';

const CASE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e53';
const ACTION_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e54';
const CATALOG_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e55';
const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e56';
const GRANT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e57';
const PRINCIPAL_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e58';

describe('ControlPlaneModerationClient', () => {
  it('通过单一能力投影发现治理权限且拒绝额外字段', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(jsonResponse({ canModerateRoom: false, canReadAudit: true }));
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });

    await expect(client.inspectCapabilities(CATALOG_ID)).resolves.toEqual({
      ok: true,
      value: { canModerateRoom: false, canReadAudit: true },
    });
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `https://control.agent-room.test/rooms/${CATALOG_ID}/moderation/capabilities`,
    );
    expect((fetchMock.mock.calls[0]?.[1] as RequestInit | undefined)?.method).toBe('GET');

    fetchMock.mockResolvedValueOnce(
      jsonResponse({ canModerateRoom: false, canReadAudit: true, role: 'audit_reader' }),
    );
    await expect(client.inspectCapabilities(CATALOG_ID)).resolves.toEqual({
      error: { code: 'moderation.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('只提交调用者明确提供的举报摘录并携带幂等案件标识', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      jsonResponse({
        caseId: CASE_ID,
        createdAtUnixMs: 1_800_000_000_000,
        description: '可疑消息',
        evidence: {
          endToEndEncrypted: true,
          matrixEventId: '$event:matrix.test',
          reporterSubmittedExcerpt: '用户明确选择的预览摘要',
          roomCatalogId: CATALOG_ID,
        },
        reason: 'malicious_content',
        resolvedAtUnixMs: null,
        state: 'open',
        targetKind: 'event',
        targetReference: '$event:matrix.test',
      }),
    );
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });

    const result = await client.report(CASE_ID, {
      description: '可疑消息',
      evidence: {
        endToEndEncrypted: true,
        matrixEventId: '$event:matrix.test',
        reporterSubmittedExcerpt: '用户明确选择的预览摘要',
        roomCatalogId: CATALOG_ID,
      },
      reason: 'malicious_content',
      targetKind: 'event',
      targetReference: '$event:matrix.test',
    });

    expect(result.ok).toBe(true);
    const request = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(new Headers(request.headers).get('Idempotency-Key')).toBe(CASE_ID);
    expect(typeof request.body).toBe('string');
    if (typeof request.body !== 'string') {
      throw new Error('举报请求必须序列化为 JSON 字符串。');
    }
    const body: unknown = JSON.parse(request.body);
    expect(body).toEqual({
      description: '可疑消息',
      evidence: {
        endToEndEncrypted: true,
        matrixEventId: '$event:matrix.test',
        reporterSubmittedExcerpt: '用户明确选择的预览摘要',
        roomCatalogId: CATALOG_ID,
      },
      reason: 'malicious_content',
      targetKind: 'event',
      targetReference: '$event:matrix.test',
    });
  });

  it('拒绝动作目标偷换且不会发起网络请求', async () => {
    const fetchMock = vi.fn();
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });

    await expect(
      client.applyAction(ACTION_ID, CATALOG_ID, {
        impactAcknowledged: true,
        kind: 'hide',
        reason: 'spam',
        targetKind: 'principal',
        targetReference: CASE_ID,
      }),
    ).resolves.toEqual({
      error: { code: 'moderation.invalid_action_input', retryable: false },
      ok: false,
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('审计响应携带正文即按隐私边界失败关闭', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      jsonResponse({
        events: [
          {
            action: 'moderation.action.applied',
            actorPrincipalId: CASE_ID,
            body: '绝不能出现的正文',
            correlationId: '0198b601-77a1-7bb8-83eb-a8fe68c97e56',
            eventId: ACTION_ID,
            occurredAtUnixMs: 1_800_000_000_000,
            outcome: 'allowed',
            reason: 'spam',
            roomCatalogId: CATALOG_ID,
            targetKind: 'event',
            targetReference: '$event:matrix.test',
          },
        ],
      }),
    );
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });

    await expect(client.listAudit(CATALOG_ID)).resolves.toEqual({
      error: { code: 'moderation.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('保留限速重试时间和相关标识', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          code: 'moderation.rate_limited',
          correlationId: 'correlation-id',
          retryable: true,
        }),
        {
          headers: { 'Content-Type': 'application/json', 'Retry-After': '42' },
          status: 429,
        },
      ),
    );
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });

    const result = await client.report(CASE_ID, {
      description: '',
      evidence: { endToEndEncrypted: false, matrixEventId: '$event:matrix.test' },
      reason: 'spam',
      targetKind: 'event',
      targetReference: '$event:matrix.test',
    });

    expect(result).toEqual({
      error: {
        code: 'moderation.rate_limited',
        correlationId: 'correlation-id',
        retryAfterSeconds: 42,
        retryable: true,
      },
      ok: false,
    });
  });
});

describe('ControlPlaneModerationClient 交给 Agent 管', () => {
  it('给授权：去掉规则两头的空白，拒绝没勾的，回来的授权多一个字段就不认', async () => {
    const granted = {
      actions: ['mute'],
      agentId: AGENT_ID,
      agentName: 'Moderator Agent',
      createdAtUnixMs: 1_800_000_000_000,
      expiresAtUnixMs: 1_802_592_000_000,
      grantedBy: PRINCIPAL_ID,
      id: GRANT_ID,
      rules: 'No ads.',
    };
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(granted));
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });
    const input = {
      actions: ['mute' as const],
      agentId: AGENT_ID,
      lifetimeSeconds: 3_600,
      rules: '  No ads.  ',
    };

    await expect(client.grant(CATALOG_ID, input)).resolves.toEqual({ ok: true, value: granted });
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      `https://control.agent-room.test/private-rooms/${CATALOG_ID}/moderation-grants`,
    );
    const request = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(request.method).toBe('POST');
    expect(jsonBody(request)).toEqual({ ...input, rules: 'No ads.' });

    await expect(client.grant(CATALOG_ID, { ...input, actions: [] })).resolves.toEqual({
      error: { code: 'moderation.invalid_grant_input', retryable: false },
      ok: false,
    });
    expect(fetchMock).toHaveBeenCalledOnce();

    fetchMock.mockResolvedValueOnce(jsonResponse({ grants: [{ ...granted, scope: 'all' }] }));
    await expect(client.listGrants(CATALOG_ID)).resolves.toEqual({
      error: { code: 'moderation.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('撤掉授权回 204、一次全撤带上确认过影响，都走对的路径', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        jsonResponse({
          failed: [{ actionId: GRANT_ID, code: 'moderation.dependency_unavailable' }],
          reversed: [AGENT_ID],
        }),
      );
    const client = new ControlPlaneModerationClient({
      baseUrl: 'https://control.agent-room.test',
      fetch: fetchMock,
    });

    await expect(client.revokeGrant(CATALOG_ID, GRANT_ID)).resolves.toEqual({
      ok: true,
      value: undefined,
    });
    await expect(client.reverseGrant(CATALOG_ID, GRANT_ID)).resolves.toEqual({
      ok: true,
      value: {
        failed: [{ actionId: GRANT_ID, code: 'moderation.dependency_unavailable' }],
        reversed: [AGENT_ID],
      },
    });

    const revoke = fetchMock.mock.calls[0] as [URL, RequestInit];
    const reverse = fetchMock.mock.calls[1] as [URL, RequestInit];
    expect(revoke[0].href).toBe(
      `https://control.agent-room.test/private-rooms/${CATALOG_ID}/moderation-grants/${GRANT_ID}`,
    );
    expect(revoke[1].method).toBe('DELETE');
    expect(reverse[0].href).toBe(
      `https://control.agent-room.test/rooms/${CATALOG_ID}/moderation/grants/${GRANT_ID}/reverse`,
    );
    expect(reverse[1].method).toBe('POST');
    expect(jsonBody(reverse[1])).toEqual({ impactAcknowledged: true });
  });
});

function jsonBody(init: RequestInit): unknown {
  if (typeof init.body !== 'string') {
    throw new Error('请求正文必须序列化为 JSON 字符串。');
  }
  return JSON.parse(init.body);
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
    status: 200,
  });
}
