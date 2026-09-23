import { describe, expect, it, vi } from 'vitest';

import { ControlPlanePrivateRoomClient } from './control-plane-private-room-client';

const ROOM = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: 'Private review',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
      status: 'joined',
    },
  ],
  name: 'Architecture room',
  ownerPrincipalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 1,
} as const;

describe('ControlPlanePrivateRoomClient', () => {
  it('校验权威列表响应而不是信任任意 JSON', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ rooms: [ROOM] }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const result = await client.list();

    expect(result.ok).toBe(true);
    expect(result.ok ? result.value[0]?.matrixRoomId : null).toBe('!private:matrix.test');
    expect(fetch).toHaveBeenCalledWith(
      new URL('https://control.agent-room.test/private-rooms'),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('创建携带 UUIDv7 幂等键与严格权限载荷', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json(ROOM, { status: 201 }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const result = await client.create(ROOM.catalogId, {
      description: 'Private review',
      invitations: [],
      name: 'Architecture room',
      retentionDays: 30,
    });

    expect(result.ok).toBe(true);
    const [, init] = fetch.mock.calls[0] ?? [];
    expect(new Headers(init?.headers).get('Idempotency-Key')).toBe(ROOM.catalogId);
    const body = init?.body;
    expect(typeof body).toBe('string');
    if (typeof body !== 'string') {
      throw new TypeError('创建请求正文必须是 JSON 字符串');
    }
    expect(JSON.parse(body)).toEqual({
      description: 'Private review',
      invitations: [],
      name: 'Architecture room',
      retentionDays: 30,
    });
  });

  it('改名用 PUT 发送名字并返回权威房间', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ ...ROOM, name: 'Design review' }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const result = await client.rename(ROOM.catalogId, 'Design review');

    expect(result.ok ? result.value.name : null).toBe('Design review');
    expect(fetch).toHaveBeenCalledWith(
      new URL(`https://control.agent-room.test/private-rooms/${ROOM.catalogId}/name`),
      expect.objectContaining({
        body: JSON.stringify({ name: 'Design review' }),
        credentials: 'include',
        method: 'PUT',
      }),
    );
  });

  it('Agent 口令：查看只有创建时间，生成时才拿到口令，停用与移出没有正文', async () => {
    const agentId = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(
        Response.json({
          agents: [
            {
              agentId,
              displayName: 'Scout',
              joinedAtUnixMs: 1_700_000_000_000,
              ownerDisplayName: null,
              status: 'joined',
              statusChangedAtUnixMs: 1_700_000_000_000,
            },
          ],
          joinCode: { createdAtUnixMs: 1_700_000_000_000 },
        }),
      )
      .mockResolvedValueOnce(
        Response.json({ code: 'K7P3-Q9XW-2DMA', createdAtUnixMs: 1_700_000_100_000 }),
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const base = `https://control.agent-room.test/private-rooms/${ROOM.catalogId}/agent-access`;

    const access = await client.agentAccess(ROOM.catalogId);
    expect(access.ok ? access.value.agents[0]?.displayName : null).toBe('Scout');
    expect(access.ok ? access.value.joinCode : null).toEqual({
      createdAtUnixMs: 1_700_000_000_000,
    });
    const generated = await client.generateJoinCode(ROOM.catalogId);
    expect(generated).toEqual({
      ok: true,
      value: { code: 'K7P3-Q9XW-2DMA', createdAtUnixMs: 1_700_000_100_000 },
    });
    expect(await client.disableJoinCode(ROOM.catalogId)).toEqual({ ok: true, value: undefined });
    expect(await client.removeCodeAgent(ROOM.catalogId, agentId)).toEqual({
      ok: true,
      value: undefined,
    });
    for (const [index, [path, method]] of [
      [base, 'GET'],
      [`${base}/code`, 'PUT'],
      [`${base}/code`, 'DELETE'],
      [`${base}/agents/${agentId}`, 'DELETE'],
    ].entries()) {
      expect(fetch).toHaveBeenNthCalledWith(
        index + 1,
        new URL(path ?? ''),
        expect.objectContaining({ credentials: 'include', method }),
      );
    }
  });

  it('不是口令格式的生成结果当作无效响应', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ code: 'OIL0-UUUU-0000', createdAtUnixMs: 1 }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.generateJoinCode(ROOM.catalogId)).toEqual({
      error: { code: 'private_room.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('保留结构化失败与关联标识', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'private_room.forbidden', correlationId: ROOM.catalogId, retryable: false },
          { status: 403 },
        ),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const result = await client.inspect(ROOM.catalogId);

    expect(result).toEqual({
      error: {
        code: 'private_room.forbidden',
        correlationId: ROOM.catalogId,
        retryable: false,
      },
      ok: false,
    });
  });
});
