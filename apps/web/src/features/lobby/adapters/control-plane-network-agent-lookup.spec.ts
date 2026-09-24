import { describe, expect, it, vi } from 'vitest';
import { ControlPlaneNetworkAgentLookup } from './control-plane-network-agent-lookup';

const network = '01990d9e-8400-7000-8000-000000000001';
const local = '01990d9e-8400-7000-8000-000000000002';

describe('网络 Agent 查询', () => {
  it('用当前登录会话一次问一批，只把服务器确认的当作网络 Agent', async () => {
    const request = vi
      .fn<typeof fetch>()
      .mockResolvedValue(
        new Response(JSON.stringify({ schemaVersion: 1, networkAgentIds: [network] })),
      );
    const client = new ControlPlaneNetworkAgentLookup({
      baseUrl: 'https://app.test/_agent-room/api',
      fetch: request,
    });

    const result = await client.lookup([network, local]);

    expect(result).toEqual({ ok: true, value: new Set([network]) });
    expect(request).toHaveBeenCalledWith(
      new URL(
        `https://app.test/_agent-room/api/network-agents/lookup?agentIds=${network}%2C${local}`,
      ),
      expect.objectContaining({ credentials: 'include', cache: 'no-store' }),
    );
  });

  it.each([
    ['服务器拒绝', () => Promise.resolve(new Response('', { status: 401 })), 'unavailable'],
    ['网络断了', () => Promise.reject(new TypeError('offline')), 'unavailable'],
    [
      '返回格式不对',
      () => Promise.resolve(new Response(JSON.stringify({ networkAgentIds: ['nope'] }))),
      'invalid_response',
    ],
  ] as const)('%s时不当作查到了', async (_case, respond, code) => {
    const client = new ControlPlaneNetworkAgentLookup({
      baseUrl: 'https://app.test/_agent-room/api',
      fetch: vi.fn<typeof fetch>(respond),
    });

    expect(await client.lookup([network])).toEqual({ ok: false, error: { code } });
  });
});
