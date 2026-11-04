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

  it('房间是谁建的：GET 单独的接口，严格校验回答', async () => {
    const origin = {
      createdByAgent: { agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48', displayName: 'Visitor' },
      ownerIsNetworkAgent: true,
    };
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json(origin))
      .mockResolvedValueOnce(Response.json({ ...origin, ownerIsNetworkAgent: 'yes' }))
      .mockResolvedValueOnce(Response.json({ ...origin, extra: true }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.origin(ROOM.catalogId)).toEqual({ ok: true, value: origin });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`https://control.agent-room.test/private-rooms/${ROOM.catalogId}/origin`),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
    for (let attempt = 0; attempt < 2; attempt += 1) {
      expect(await client.origin(ROOM.catalogId)).toEqual({
        ok: false,
        error: { code: 'private_room.invalid_response', retryable: false },
      });
    }
  });

  it('成员的昵称：GET 单独的接口，严格校验回答', async () => {
    const people = [{ displayName: 'Ada', principalId: ROOM.ownerPrincipalId }];
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ people }))
      .mockResolvedValueOnce(Response.json({ people: [{ ...people[0], email: 'a@b.test' }] }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.people(ROOM.catalogId)).toEqual({ ok: true, value: people });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`https://control.agent-room.test/private-rooms/${ROOM.catalogId}/people`),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
    expect(await client.people(ROOM.catalogId)).toEqual({
      ok: false,
      error: { code: 'private_room.invalid_response', retryable: false },
    });
  });

  it('解除封禁用 POST，返回权威房间', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(ROOM));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const banned = '0198b601-77a1-7bb8-83eb-a8fe68c97e4a';

    const result = await client.unban(ROOM.catalogId, banned);

    expect(result.ok).toBe(true);
    expect(fetch).toHaveBeenCalledWith(
      new URL(
        `https://control.agent-room.test/private-rooms/${ROOM.catalogId}/members/${banned}/unban`,
      ),
      expect.objectContaining({ credentials: 'include', method: 'POST' }),
    );
  });

  it('改公告用 PUT 发正文，回写进去的那一版；限速时把错误码交给界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(
        Response.json({ text: 'Reply in English.', updatedAtUnixMs: 1_793_606_400_000 }),
      )
      .mockResolvedValueOnce(
        Response.json(
          { code: 'room_notice.rate_limited', retryable: true },
          { headers: { 'Retry-After': '60' }, status: 429 },
        ),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const written = await client.updateNotice(ROOM.catalogId, 'Reply in English.');
    const limited = await client.updateNotice(ROOM.catalogId, '');

    expect(written).toEqual({
      ok: true,
      value: { text: 'Reply in English.', updatedAtUnixMs: 1_793_606_400_000 },
    });
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      new URL(`https://control.agent-room.test/private-rooms/${ROOM.catalogId}/notice`),
      expect.objectContaining({
        body: JSON.stringify({ text: 'Reply in English.' }),
        method: 'PUT',
      }),
    );
    expect(limited.ok ? null : limited.error.code).toBe('room_notice.rate_limited');
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

  it('Agent 口令：口令状态只有创建时间，生成和再看拿到口令，停用与移出没有正文', async () => {
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
    const code = {
      ok: true,
      value: { code: 'K7P3-Q9XW-2DMA', createdAtUnixMs: 1_700_000_100_000 },
    };
    expect(await client.generateJoinCode(ROOM.catalogId)).toEqual(code);
    expect(await client.revealJoinCode(ROOM.catalogId)).toEqual(code);
    expect(await client.disableJoinCode(ROOM.catalogId)).toEqual({ ok: true, value: undefined });
    expect(await client.removeCodeAgent(ROOM.catalogId, agentId)).toEqual({
      ok: true,
      value: undefined,
    });
    for (const [index, [path, method]] of [
      [base, 'GET'],
      [`${base}/code`, 'PUT'],
      [`${base}/code`, 'GET'],
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

  it('敲门：单独一个接口列出在等的，放行用 PUT 拿回进来的 Agent，不让进用 DELETE', async () => {
    const agentId = '0198b601-77a1-7bb8-83eb-a8fe68c97e51';
    const knock = {
      agentId,
      displayName: 'Sol',
      expiresAtUnixMs: 1_700_003_600_000,
      knockedAtUnixMs: 1_700_000_000_000,
    };
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ knocks: [knock] }))
      .mockResolvedValueOnce(
        Response.json({
          agent: {
            agentId,
            displayName: 'Sol',
            joinedAtUnixMs: 1_700_000_060_000,
            ownerDisplayName: null,
            status: 'joined',
            statusChangedAtUnixMs: 1_700_000_060_000,
          },
        }),
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const base = `https://control.agent-room.test/private-rooms/${ROOM.catalogId}/agent-access`;

    expect(await client.agentKnocks(ROOM.catalogId)).toEqual({ ok: true, value: [knock] });
    const admitted = await client.admitKnock(ROOM.catalogId, agentId);
    expect(admitted.ok ? admitted.value.joinedAtUnixMs : null).toBe(1_700_000_060_000);
    expect(await client.declineKnock(ROOM.catalogId, agentId)).toEqual({
      ok: true,
      value: undefined,
    });
    for (const [index, [path, method]] of [
      [`${base}/knocks`, 'GET'],
      [`${base}/agents/${agentId}`, 'PUT'],
      [`${base}/knocks/${agentId}`, 'DELETE'],
    ].entries()) {
      expect(fetch).toHaveBeenNthCalledWith(
        index + 1,
        new URL(path ?? ''),
        expect.objectContaining({ credentials: 'include', method }),
      );
    }
  });

  it('放行时服务器要替它进加密房间，比平常的请求多等一会', async () => {
    vi.useFakeTimers();
    try {
      const fetch = vi.fn<typeof globalThis.fetch>(
        (_input, init) =>
          new Promise((_resolve, reject) => {
            init?.signal?.addEventListener('abort', () => {
              reject(new DOMException('aborted', 'AbortError'));
            });
          }),
      );
      const client = new ControlPlanePrivateRoomClient({
        baseUrl: 'https://control.agent-room.test',
        fetch,
        timeoutMs: 1_000,
      });
      let settled = false;
      const admitted = client
        .admitKnock(ROOM.catalogId, '0198b601-77a1-7bb8-83eb-a8fe68c97e51')
        .finally(() => {
          settled = true;
        });

      await vi.advanceTimersByTimeAsync(30_000);
      expect(settled).toBe(false);
      await vi.advanceTimersByTimeAsync(15_000);
      expect(await admitted).toEqual({
        error: { code: 'private_room.unreachable', retryable: true },
        ok: false,
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('看不懂的敲门列表当作无效响应', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ knocks: [{ agentId: 'not-a-uuid', displayName: '' }] }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.agentKnocks(ROOM.catalogId)).toEqual({
      error: { code: 'private_room.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('口令看不到时把服务器的错误码交给界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json({ code: 'join_code.not_revealable', retryable: false }, { status: 404 }),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.revealJoinCode(ROOM.catalogId)).toEqual({
      error: { code: 'join_code.not_revealable', retryable: false },
      ok: false,
    });
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

describe('ControlPlanePrivateRoomClient 的邀请链接', () => {
  const SECRET = '0123456789abcdefghjkmnpqrs';
  const LINK = {
    createdAtUnixMs: 1_780_000_000_000,
    expiresAtUnixMs: 1_780_604_800_000,
    permissions: { capabilities: ['view', 'speak', 'automate'] },
    secret: SECRET,
    url: `https://agentroom.chat/join#${SECRET}`,
  };

  it('生成时带上权限，不带就让服务器给默认的；再看、停用走同一个地址', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockImplementation(() => Promise.resolve(Response.json(LINK, { status: 201 })));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const path = new URL(
      `https://control.agent-room.test/private-rooms/${ROOM.catalogId}/invite-link`,
    );

    expect((await client.createInviteLink(ROOM.catalogId)).ok).toBe(true);
    expect(fetch).toHaveBeenLastCalledWith(
      path,
      expect.objectContaining({ body: '{}', method: 'POST' }),
    );
    await client.createInviteLink(ROOM.catalogId, { capabilities: ['view', 'speak'] });
    expect(fetch).toHaveBeenLastCalledWith(
      path,
      expect.objectContaining({
        body: JSON.stringify({ permissions: { capabilities: ['view', 'speak'] } }),
      }),
    );
    expect(await client.inviteLink(ROOM.catalogId)).toEqual({ ok: true, value: LINK });
    expect(fetch).toHaveBeenLastCalledWith(path, expect.objectContaining({ method: 'GET' }));
    fetch.mockResolvedValueOnce(new Response(null, { status: 204 }));
    expect(await client.disableInviteLink(ROOM.catalogId)).toEqual({ ok: true, value: undefined });
    expect(fetch).toHaveBeenLastCalledWith(path, expect.objectContaining({ method: 'DELETE' }));
  });

  it('打开、兑换链接时秘密只放在正文里，不进地址', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(
        Response.json({
          catalogId: ROOM.catalogId,
          expiresAtUnixMs: LINK.expiresAtUnixMs,
          inviterDisplayName: 'Ada',
          membership: 'absent',
          permissions: LINK.permissions,
          roomName: 'Architecture room',
        }),
      )
      .mockResolvedValueOnce(Response.json({ catalogId: ROOM.catalogId, membership: 'invited' }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const preview = await client.previewInvite(SECRET);
    const redeemed = await client.redeemInvite(SECRET);

    expect(preview.ok && preview.value.inviterDisplayName).toBe('Ada');
    expect(redeemed).toEqual({
      ok: true,
      value: { catalogId: ROOM.catalogId, membership: 'invited' },
    });
    for (const [url, init] of fetch.mock.calls) {
      expect(href(url)).not.toContain(SECRET);
      expect(init?.body).toBe(JSON.stringify({ secret: SECRET }));
    }
    expect(fetch.mock.calls.map(([url]) => href(url))).toEqual([
      'https://control.agent-room.test/private-room-invites/preview',
      'https://control.agent-room.test/private-room-invites/redeem',
    ]);
  });

  it('链接失效时保留服务器的错误码', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json({ code: 'invite_link.not_found', retryable: false }, { status: 404 }),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.previewInvite(SECRET)).toEqual({
      error: { code: 'invite_link.not_found', retryable: false },
      ok: false,
    });
  });
});

describe('ControlPlanePrivateRoomClient 的公开设置', () => {
  const LISTING = {
    allowNetworkAgents: true,
    endToEndEncrypted: false,
    firstListedAtUnixMs: 1_780_000_000_000,
    joinMode: 'request',
    listed: true,
    memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
    platformHold: null,
    shareCode: 'aB3xYz',
    shareUrl: 'https://agentroom.chat/r/aB3xYz',
  } as const;

  it('读、改、换码走同一组地址，严格校验回答', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockImplementation(() => Promise.resolve(Response.json(LISTING)));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const path = `https://control.agent-room.test/private-rooms/${ROOM.catalogId}/listing`;

    expect(await client.listing(ROOM.catalogId)).toEqual({ ok: true, value: LISTING });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(path),
      expect.objectContaining({ method: 'GET' }),
    );

    await client.updateListing(ROOM.catalogId, {
      allowNetworkAgents: false,
      joinMode: 'open',
      listed: true,
      memberPermissions: { capabilities: ['view', 'speak'] },
    });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(path),
      expect.objectContaining({
        body: JSON.stringify({
          allowNetworkAgents: false,
          joinMode: 'open',
          listed: true,
          memberPermissions: { capabilities: ['view', 'speak'] },
        }),
        method: 'PUT',
      }),
    );

    expect((await client.changeShareCode(ROOM.catalogId)).ok).toBe(true);
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(`${path}/share-code`),
      expect.objectContaining({ method: 'POST' }),
    );

    fetch.mockResolvedValueOnce(Response.json({ ...LISTING, extra: true }));
    expect(await client.listing(ROOM.catalogId)).toEqual({
      ok: false,
      error: { code: 'private_room.invalid_response', retryable: false },
    });
  });

  it('一个账户只能有一间公开房间：把服务器的错误码交给界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'room_listing.limit_reached', correlationId: 'c-1', retryable: false },
          { status: 409 },
        ),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.updateListing(ROOM.catalogId, { ...LISTING, joinMode: 'open' })).toEqual({
      ok: false,
      error: { code: 'room_listing.limit_reached', correlationId: 'c-1', retryable: false },
    });
  });
});

describe('ControlPlanePrivateRoomClient 的申请', () => {
  const REQUESTER = '0198b601-77a1-7bb8-83eb-a8fe68c97e50';
  const REQUEST = {
    displayName: 'Lin',
    message: 'Can I bring my agent?',
    principalId: REQUESTER,
    requestedAtUnixMs: 1_780_000_000_000,
  } as const;

  it('列出在等的申请，批准和拒绝用 PUT 发决定', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ requests: [REQUEST] }))
      .mockResolvedValueOnce(Response.json({ status: 'approved' }))
      .mockResolvedValueOnce(Response.json({ status: 'rejected' }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const path = `https://control.agent-room.test/private-rooms/${ROOM.catalogId}/join-requests`;

    expect(await client.joinRequests(ROOM.catalogId)).toEqual({ ok: true, value: [REQUEST] });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(path),
      expect.objectContaining({ method: 'GET' }),
    );

    expect(await client.answerJoinRequest(ROOM.catalogId, REQUESTER, 'approve')).toEqual({
      ok: true,
      value: undefined,
    });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(`${path}/${REQUESTER}`),
      expect.objectContaining({ body: JSON.stringify({ decision: 'approve' }), method: 'PUT' }),
    );

    await client.answerJoinRequest(ROOM.catalogId, REQUESTER, 'reject');
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(`${path}/${REQUESTER}`),
      expect.objectContaining({ body: JSON.stringify({ decision: 'reject' }), method: 'PUT' }),
    );
  });

  it('看不懂的申请列表当作无效响应，被封禁的人批准不了时交出服务器的错误码', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ requests: [{ ...REQUEST, email: 'x' }] }))
      .mockResolvedValueOnce(
        Response.json(
          { code: 'room_join.banned', correlationId: 'c-2', retryable: false },
          { status: 409 },
        ),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    expect(await client.joinRequests(ROOM.catalogId)).toEqual({
      ok: false,
      error: { code: 'private_room.invalid_response', retryable: false },
    });
    expect(await client.answerJoinRequest(ROOM.catalogId, REQUESTER, 'approve')).toEqual({
      ok: false,
      error: { code: 'room_join.banned', correlationId: 'c-2', retryable: false },
    });
  });
});

function href(target: Parameters<typeof globalThis.fetch>[0]): string {
  return typeof target === 'string' ? target : target instanceof URL ? target.href : target.url;
}

describe('ControlPlanePrivateRoomClient 从发现页进公开房间', () => {
  it('直接进、申请（带上留言）、撤回自己的申请', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ catalogId: ROOM.catalogId, membership: 'invited' }))
      .mockResolvedValueOnce(
        Response.json({
          catalogId: ROOM.catalogId,
          membership: 'pending',
          requestedAtUnixMs: 1_780_000_000_000,
        }),
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });
    const room = `https://control.agent-room.test/private-rooms/${ROOM.catalogId}`;

    expect(await client.join(ROOM.catalogId)).toEqual({
      ok: true,
      value: { catalogId: ROOM.catalogId, membership: 'invited' },
    });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(`${room}/join`),
      expect.objectContaining({ method: 'POST' }),
    );

    expect(await client.requestToJoin(ROOM.catalogId, 'Hi')).toEqual({
      ok: true,
      value: {
        catalogId: ROOM.catalogId,
        membership: 'pending',
        requestedAtUnixMs: 1_780_000_000_000,
      },
    });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(`${room}/join-requests`),
      expect.objectContaining({ body: JSON.stringify({ message: 'Hi' }), method: 'POST' }),
    );

    expect(await client.withdrawJoinRequest(ROOM.catalogId)).toEqual({
      ok: true,
      value: undefined,
    });
    expect(fetch).toHaveBeenLastCalledWith(
      new URL(`${room}/join-requests`),
      expect.objectContaining({ method: 'DELETE' }),
    );
  });

  it('要申请的房间直接进时交出服务器的错误码', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json({ code: 'room_join.approval_required', retryable: false }, { status: 409 }),
      );
    const client = new ControlPlanePrivateRoomClient({
      baseUrl: 'https://control.agent-room.test',
      fetch,
    });

    const joined = await client.join(ROOM.catalogId);
    expect(joined.ok ? null : joined.error.code).toBe('room_join.approval_required');
  });
});
