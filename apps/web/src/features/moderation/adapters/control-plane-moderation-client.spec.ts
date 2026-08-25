import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneModerationClient } from './control-plane-moderation-client';

const CASE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e53';
const ACTION_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e54';
const CATALOG_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e55';

describe('ControlPlaneModerationClient', () => {
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
    expect(JSON.parse(String(request.body))).toEqual({
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

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
    status: 200,
  });
}
