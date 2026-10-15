import { describe, expect, it, vi } from 'vitest';
import { ControlPlaneAgentOwnerLookup } from './control-plane-agent-owner-lookup';

const room = '01990d9e-8400-7000-8000-000000000401';
const local = '01990d9e-8400-7000-8000-000000000001';
const network = '01990d9e-8400-7000-8000-000000000002';
const orphan = '01990d9e-8400-7000-8000-000000000003';
const ada = { displayName: 'Ada', principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42' };

describe('Agent 是谁的：查询', () => {
  it('用当前登录会话按房间问一批，没有的字段不写出来', async () => {
    const request = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          agents: [
            { agentId: local, kind: 'local', owner: ada },
            { agentId: network, kind: 'network', admittedBy: ada },
            { agentId: orphan, kind: 'local' },
          ],
        }),
      ),
    );
    const client = new ControlPlaneAgentOwnerLookup({
      baseUrl: 'https://app.test/_agent-room/api',
      fetch: request,
    });

    const result = await client.lookup(room, [local, network, orphan]);

    expect(result).toEqual({
      ok: true,
      value: [
        { agentId: local, kind: 'local', owner: ada },
        { agentId: network, kind: 'network', admittedBy: ada },
        { agentId: orphan, kind: 'local' },
      ],
    });
    expect(result.ok && 'owner' in (result.value[2] ?? {})).toBe(false);
    expect(request).toHaveBeenCalledWith(
      new URL(
        `https://app.test/_agent-room/api/rooms/${room}/agent-owners?agentIds=${local}%2C${network}%2C${orphan}`,
      ),
      expect.objectContaining({ credentials: 'include', cache: 'no-store' }),
    );
  });

  it.each([
    ['服务器拒绝', () => Promise.resolve(new Response('', { status: 403 })), 'unavailable'],
    ['网络断了', () => Promise.reject(new TypeError('offline')), 'unavailable'],
    [
      '多了不认识的字段',
      () =>
        Promise.resolve(
          new Response(
            JSON.stringify({ agents: [{ agentId: local, kind: 'local', email: 'a@b.test' }] }),
          ),
        ),
      'invalid_response',
    ],
    [
      '本机 Agent 带着放行的人',
      () =>
        Promise.resolve(
          new Response(
            JSON.stringify({ agents: [{ agentId: local, kind: 'local', admittedBy: ada }] }),
          ),
        ),
      'invalid_response',
    ],
  ] as const)('%s时不当作查到了', async (_case, respond, code) => {
    const client = new ControlPlaneAgentOwnerLookup({
      baseUrl: 'https://app.test/_agent-room/api',
      fetch: vi.fn<typeof fetch>(respond),
    });

    expect(await client.lookup(room, [local])).toEqual({ ok: false, error: { code } });
  });
});
