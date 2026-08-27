import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneOnboardingClient } from '@/features/onboarding/adapters/control-plane-onboarding-client';

const agent = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  avatarContentId: null,
  description: '',
  displayName: 'First Agent',
  matrixUserId: '@agent:matrix.test',
  registeredAtUnixMs: 1,
  slug: 'first-agent',
  visibility: 'private',
};

describe('控制面首次引导适配器', () => {
  it('用同源会话读取 Agent，并以 PUT 收敛默认 Agent', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ agents: [agent] }))
      .mockResolvedValueOnce(Response.json(agent));
    const client = new ControlPlaneOnboardingClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(client.listAgents()).resolves.toEqual({ ok: true, value: [agent] });
    await expect(client.ensureDefaultAgent()).resolves.toEqual({ ok: true, value: agent });
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      new URL('https://api.agent-room.test/agents'),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
    expect(fetch).toHaveBeenNthCalledWith(
      2,
      new URL('https://api.agent-room.test/onboarding/default-agent'),
      expect.objectContaining({ credentials: 'include', method: 'PUT' }),
    );
  });

  it('拒绝版本错误的标识而不把畸形响应交给 UI', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      Response.json({
        agents: [{ ...agent, agentId: '00000000-0000-4000-8000-000000000001' }],
      }),
    );
    const client = new ControlPlaneOnboardingClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(client.listAgents()).resolves.toEqual({
      error: { code: 'onboarding.agents_response_invalid', retryable: true },
      ok: false,
    });
  });
});
