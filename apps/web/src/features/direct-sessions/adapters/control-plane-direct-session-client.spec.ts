import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneDirectSessionClient } from './control-plane-direct-session-client';

const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e52';
const CATALOG_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e53';

describe('ControlPlaneDirectSessionClient', () => {
  it('使用同源凭据打开稳定直接会话', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      Response.json(sessionPayload(), {
        headers: { 'content-type': 'application/json' },
        status: 200,
      }),
    );
    const client = new ControlPlaneDirectSessionClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const result = await client.open(AGENT_ID);

    expect(result).toMatchObject({ ok: true, value: { catalogId: CATALOG_ID } });
    const [input, init] = fetch.mock.calls[0] ?? [];
    expect(String(input)).toBe('https://control.agent-room.test/direct-sessions');
    expect(init).toMatchObject({ cache: 'no-store', credentials: 'include', method: 'POST' });
    expect(init?.body).toBe(JSON.stringify({ targetAgentId: AGENT_ID }));
  });

  it('拒绝生命周期与房间实例互相矛盾的响应', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json({ ...sessionPayload(), matrixRoomId: null }, { status: 200 }),
      );
    const client = new ControlPlaneDirectSessionClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    await expect(client.inspect(CATALOG_ID)).resolves.toEqual({
      error: { code: 'direct_session.invalid_response', retryable: false },
      ok: false,
    });
  });
});

function sessionPayload() {
  return {
    catalogId: CATALOG_ID,
    contactPolicy: {
      agentBlocksPrincipal: false,
      deliveryAllowed: true,
      presenceDisclosure: 'coarse',
      principalBlocksAgent: false,
    },
    lifecycle: 'active',
    matrixRoomId: '!direct:matrix.agent-room.test',
    roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e54',
    target: {
      agentId: AGENT_ID,
      avatarContentId: null,
      displayName: 'Build Agent',
      matrixUserId: '@_agent_build:matrix.agent-room.test',
    },
    version: 1,
  };
}
