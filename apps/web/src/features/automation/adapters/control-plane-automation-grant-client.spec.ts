import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneAutomationGrantClient } from './control-plane-automation-grant-client';
import type { CreateAutomationGrantInput } from '@/features/automation/domain/automation-grant';

const GRANT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';
const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const INSTANCE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const ROOM_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';

describe('ControlPlaneAutomationGrantClient', () => {
  it('创建请求携带同源会话、幂等键与严格正文', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(jsonResponse(grant()));
    const client = new ControlPlaneAutomationGrantClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    const result = await client.create(GRANT_ID, creationInput());

    expect(result.ok).toBe(true);
    expect(fetch).toHaveBeenCalledOnce();
    const [target, init] = fetch.mock.calls[0] ?? [];
    expect(String(target)).toBe('https://api.agent-room.test/automation-grants');
    expect(init?.credentials).toBe('include');
    expect(new Headers(init?.headers).get('Idempotency-Key')).toBe(GRANT_ID);
    expect(JSON.parse(String(init?.body))).toEqual(creationInput());
  });

  it('列表响应包含未知字段时失败关闭', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(jsonResponse({ grants: [{ ...grant(), unexpected: true }] }));
    const client = new ControlPlaneAutomationGrantClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(client.list()).resolves.toEqual({
      error: { code: 'automation.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('撤销保留服务端错误与关联标识', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        jsonResponse(
          { code: 'authentication.recent_authentication_required', retryable: false },
          401,
          { 'x-correlation-id': '0198b601-77a1-7bb8-83eb-a8fe68c97e55' },
        ),
      );
    const client = new ControlPlaneAutomationGrantClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(client.revoke(GRANT_ID)).resolves.toEqual({
      error: {
        code: 'authentication.recent_authentication_required',
        correlationId: '0198b601-77a1-7bb8-83eb-a8fe68c97e55',
        retryable: false,
      },
      ok: false,
    });
  });

  it('本地拒绝超出硬上限的创建参数且不发请求', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>();
    const client = new ControlPlaneAutomationGrantClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    const result = await client.create(GRANT_ID, {
      ...creationInput(),
      maxMessagesPerMinute: 61,
    });

    expect(result).toEqual({
      error: { code: 'automation.invalid_creation_input', retryable: false },
      ok: false,
    });
    expect(fetch).not.toHaveBeenCalled();
  });
});

function creationInput(): CreateAutomationGrantInput {
  return {
    agentId: AGENT_ID,
    agentInstanceId: INSTANCE_ID,
    audience: 'known_room_members',
    impactAcknowledged: true,
    lifetimeSeconds: 3_600,
    maxMessagesPerMinute: 6,
    maxTotalMessages: 100,
    messageKinds: ['room_message', 'reply'],
    requiresRiskScan: true,
    roomCatalogId: ROOM_ID,
  };
}

function grant() {
  return {
    agentId: AGENT_ID,
    agentInstanceId: INSTANCE_ID,
    audience: 'known_room_members',
    expiresAtUnixMs: 1_700_003_600_000,
    grantId: GRANT_ID,
    maxMessagesPerMinute: 6,
    maxTotalMessages: 100,
    messageKinds: ['room_message', 'reply'],
    messagesInCurrentMinute: 1,
    requiresRiskScan: true,
    revokedAtUnixMs: null,
    roomCatalogId: ROOM_ID,
    startsAtUnixMs: 1_700_000_000_000,
    status: 'active',
    totalMessages: 10,
  };
}

function jsonResponse(
  value: unknown,
  status = 200,
  headers: Readonly<Record<string, string>> = {},
): Response {
  return new Response(JSON.stringify(value), {
    headers: { 'content-type': 'application/json', ...headers },
    status,
  });
}
