import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneAgentDirectoryClient } from './control-plane-agent-directory-client';

const AGENT = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  avatarContentId: null,
  description: 'Builds and verifies releases.',
  displayName: 'Build Agent',
  matrixUserId: '@_agent_build:matrix.agent-room.test',
  registeredAtUnixMs: 1_700_000_000_000,
  slug: 'build-agent',
  visibility: 'private',
} as const;

describe('ControlPlaneAgentDirectoryClient', () => {
  it('读取当前账户拥有的稳定 Agent 身份', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ agents: [AGENT] }));

    const result = await createClient(fetch).listOwnedAgents();

    expect(result).toEqual({ ok: true, value: [AGENT] });
    expect(fetch).toHaveBeenCalledWith(
      new URL('https://control.agent-room.test/agents'),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('拒绝损坏的目录响应', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ agents: [{ ...AGENT, visibility: 'invisible' }] }));

    expect(await createClient(fetch).listOwnedAgents()).toEqual({
      error: { code: 'workspace.invalid_agent_directory', retryable: false },
      ok: false,
    });
  });

  it('保留服务端关联标识和重试语义', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'authentication.session_required', retryable: false },
          { headers: { 'x-correlation-id': 'correlation-01' }, status: 401 },
        ),
      );

    expect(await createClient(fetch).listOwnedAgents()).toEqual({
      error: {
        code: 'authentication.session_required',
        correlationId: 'correlation-01',
        retryable: false,
      },
      ok: false,
    });
  });

  it('把网络异常映射为可重试失败', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockRejectedValue(new TypeError('offline'));

    expect(await createClient(fetch).listOwnedAgents()).toEqual({
      error: { code: 'workspace.unreachable', retryable: true },
      ok: false,
    });
  });
});

function createClient(fetch: typeof globalThis.fetch) {
  return new ControlPlaneAgentDirectoryClient({
    baseUrl: 'https://control.agent-room.test',
    fetch,
  });
}
