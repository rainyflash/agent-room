import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneProjectRoomClient } from './control-plane-project-room-client';

const BASE = 'https://control.agent-room.test';
const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const CLAIM = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';

const lines = {
  agentsLine:
    "Agent Room: this project's room is https://agentroom.test/r/aB3xYz. Agents join with `agent-room join https://agentroom.test/r/aB3xYz`, or over the network via https://agentroom.test/agents.txt",
  badgeMarkdown:
    '[![Agent Room](https://agentroom.test/gh/rainyflash/agent-room/badge.svg)](https://agentroom.test/r/aB3xYz)',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
};

const PENDING = {
  catalogId: CATALOG,
  claimId: CLAIM,
  claimedAtUnixMs: 1_790_000_000_000,
  lastCheck: { atUnixMs: 1_790_000_300_000, result: 'not_in_repository' },
  repository: 'rainyflash/agent-room',
  shareCode: 'aB3xYz',
  status: 'pending',
  ...lines,
};

const BINDING = {
  repository: 'rainyflash/agent-room',
  repositoryUrl: 'https://github.com/rainyflash/agent-room',
  verifiedAtUnixMs: 1_790_000_600_000,
  verifiedByAccountId: OWNER,
  verifiedFile: 'AGENTS.md',
  shareCode: 'aB3xYz',
  ...lines,
};

function client(fetch: typeof globalThis.fetch) {
  return new ControlPlaneProjectRoomClient({ baseUrl: BASE, fetch });
}

describe('ControlPlaneProjectRoomClient', () => {
  it('认领带上仓库和这间房，回答照新的 schema 严格校验', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(PENDING));

    const result = await client(fetch).claim({
      catalogId: CATALOG,
      repository: 'rainyflash/agent-room',
    });

    expect(result).toEqual({ ok: true, value: PENDING });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/project-rooms/claims`),
      expect.objectContaining({
        body: JSON.stringify({ repository: 'rainyflash/agent-room', catalogId: CATALOG }),
        credentials: 'include',
        method: 'POST',
      }),
    );
  });

  it('回答多一个字段、少一个字段、结果不认得都当成回答无效', async () => {
    for (const body of [
      { ...PENDING, extra: true },
      { ...PENDING, badgeMarkdown: undefined },
      { ...PENDING, lastCheck: { atUnixMs: 1, result: 'verified' } },
      { ...PENDING, repository: 'not a repository' },
    ]) {
      const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(body));
      const result = await client(fetch).inspectClaim(CLAIM);
      expect(result.ok ? null : result.error.code).toBe('private_room.invalid_response');
    }
  });

  it('认上以后认领是 null、绑上的那间房照样认', async () => {
    const bound = {
      ...PENDING,
      claimId: null,
      claimedAtUnixMs: null,
      lastCheck: null,
      status: 'bound',
    };
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(bound));

    const result = await client(fetch).checkClaim(CLAIM);

    expect(result).toEqual({ ok: true, value: bound });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/project-rooms/claims/${CLAIM}/check`),
      expect.objectContaining({ method: 'POST' }),
    );
  });

  it('核对太快：429 的 Retry-After 交给界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'project_room.check_too_soon', retryable: true },
          { headers: { 'Retry-After': '42' }, status: 429 },
        ),
      );

    const result = await client(fetch).checkClaim(CLAIM);

    expect(result).toEqual({
      ok: false,
      error: { code: 'project_room.check_too_soon', retryAfterSeconds: 42, retryable: true },
    });
  });

  it('回答里写着 retryAfterSeconds 时以它为准', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'project_room.check_too_soon', retryable: true, retryAfterSeconds: 17 },
          { headers: { 'Retry-After': '42' }, status: 429 },
        ),
      );

    const result = await client(fetch).checkBinding(CATALOG);

    expect(result.ok ? null : result.error.retryAfterSeconds).toBe(17);
  });

  it('撤回认领、解绑是 204；解绑超名额回 409 的错误码', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        Response.json(
          { code: 'private_room.public_limit_reached', retryable: false },
          { status: 409 },
        ),
      );

    const cancelled = await client(fetch).cancelClaim(CLAIM);
    const unbound = await client(fetch).unbind(CATALOG);

    expect(cancelled).toEqual({ ok: true, value: undefined });
    expect(unbound.ok ? null : unbound.error.code).toBe('private_room.public_limit_reached');
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      new URL(`${BASE}/project-rooms/claims/${CLAIM}`),
      expect.objectContaining({ method: 'DELETE' }),
    );
    expect(fetch).toHaveBeenNthCalledWith(
      2,
      new URL(`${BASE}/private-rooms/${CATALOG}/repository`),
      expect.objectContaining({ method: 'DELETE' }),
    );
  });

  it('没绑回 null，绑着的读出仓库、谁什么时候在哪个文件里核对上', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json(null))
      .mockResolvedValueOnce(Response.json(BINDING));

    expect(await client(fetch).binding(CATALOG)).toEqual({ ok: true, value: null });
    expect(await client(fetch).binding(CATALOG)).toEqual({ ok: true, value: BINDING });
  });

  it('绑定的仓库地址只认 github.com 的 https，文件只认那两个', async () => {
    for (const body of [
      { ...BINDING, repositoryUrl: 'https://evil.test/rainyflash/agent-room' },
      { ...BINDING, repositoryUrl: 'http://github.com/rainyflash/agent-room' },
      { ...BINDING, verifiedFile: 'CLAUDE.md' },
      { ...BINDING, extra: 1 },
    ]) {
      const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(body));
      const result = await client(fetch).binding(CATALOG);
      expect(result.ok ? null : result.error.code).toBe('private_room.invalid_response');
    }
  });

  it('自己在等的认领：读出一列，每一个都照认领的 schema 严格校验', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ claims: [PENDING] }))
      .mockResolvedValueOnce(Response.json({ claims: [] }))
      .mockResolvedValueOnce(Response.json({ claims: [{ ...PENDING, extra: 1 }] }))
      .mockResolvedValueOnce(Response.json([PENDING]));

    expect(await client(fetch).claims()).toEqual({ ok: true, value: [PENDING] });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/project-rooms/claims`),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
    expect(await client(fetch).claims()).toEqual({ ok: true, value: [] });
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const result = await client(fetch).claims();
      expect(result.ok ? null : result.error.code).toBe('private_room.invalid_response');
    }
  });

  it('房间名旁边的勾：只有仓库和 github.com 的地址，没绑回 null', async () => {
    const mark = {
      repository: 'rainyflash/agent-room',
      repositoryUrl: 'https://github.com/rainyflash/agent-room',
    };
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json(mark))
      .mockResolvedValueOnce(Response.json(null));

    expect(await client(fetch).roomRepository(CATALOG)).toEqual({ ok: true, value: mark });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/rooms/${CATALOG}/repository`),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
    expect(await client(fetch).roomRepository(CATALOG)).toEqual({ ok: true, value: null });

    for (const body of [
      { ...mark, repositoryUrl: 'https://evil.test/rainyflash/agent-room' },
      { ...mark, verifiedByAccountId: OWNER },
      { repository: 'not a repository', repositoryUrl: mark.repositoryUrl },
    ]) {
      const invalid = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(body));
      const result = await client(invalid).roomRepository(CATALOG);
      expect(result.ok ? null : result.error.code).toBe('private_room.invalid_response');
    }
  });

  it('再核对一次：读出绑定和这次的结果', async () => {
    const answer = {
      binding: BINDING,
      lastCheck: { atUnixMs: 1_790_000_900_000, result: 'not_in_repository' },
    };
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(answer));

    expect(await client(fetch).checkBinding(CATALOG)).toEqual({ ok: true, value: answer });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/private-rooms/${CATALOG}/repository/check`),
      expect.objectContaining({ method: 'POST' }),
    );
  });
});
