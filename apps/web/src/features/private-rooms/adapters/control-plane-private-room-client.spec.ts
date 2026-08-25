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
