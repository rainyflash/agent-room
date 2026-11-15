import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneSubRoomClient } from './control-plane-sub-room-client';

const BASE = 'https://control.agent-room.test';
const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const SUB = '0198b601-77a1-7bb8-83eb-a8fe68c97e50';
const PERSON = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const CATEGORY = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const REF = { parentId: PARENT, subRoomId: SUB } as const;

const ROOM = {
  catalogId: SUB,
  description: 'Release checklist',
  matrixRoomId: '!sub:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: PERSON,
      status: 'joined',
    },
  ],
  name: 'Release',
  ownerPrincipalId: PERSON,
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 1,
} as const;

const SUB_ROOM = {
  catalogId: SUB,
  name: 'Release',
  description: 'Release checklist',
  access: 'request',
  hidden: false,
  categoryId: CATEGORY,
  createdByPrincipalId: PERSON,
  createdAtUnixMs: 1_763_000_000_000,
  managerPrincipalIds: [PERSON],
  joined: true,
  banned: false,
  memberCount: 2,
  agentCount: 1,
  lastMessageAtUnixMs: 1_763_000_100_000,
  pendingRequestCount: 1,
} as const;

const CATEGORIES = [{ categoryId: CATEGORY, name: 'Work', position: 0 }] as const;

function clientAnswering(response: Response) {
  const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(response);
  return { client: new ControlPlaneSubRoomClient({ baseUrl: BASE, fetch }), fetch };
}

function sent(fetch: ReturnType<typeof vi.fn<typeof globalThis.fetch>>) {
  const [url, init] = fetch.mock.calls[0] ?? [];
  const body = init?.body;
  return {
    body: typeof body === 'string' ? (JSON.parse(body) as unknown) : undefined,
    headers: new Headers(init?.headers),
    method: init?.method,
    url: href(url),
  };
}

function href(input: Parameters<typeof globalThis.fetch>[0] | undefined): string {
  if (input === undefined || typeof input === 'string') {
    return input ?? '';
  }
  return input instanceof URL ? input.href : input.url;
}

describe('ControlPlaneSubRoomClient', () => {
  it('读名单：带着父房间和分类，回答里以后多出来的项照样读得了', async () => {
    const { client, fetch } = clientAnswering(
      Response.json({
        parent: { catalogId: PARENT, name: 'Agent Room' },
        canCreate: true,
        categories: CATEGORIES,
        subRooms: [{ ...SUB_ROOM, somethingNew: 1 }],
        alsoNew: true,
      }),
    );

    const result = await client.subRooms(SUB);

    expect(result.ok).toBe(true);
    expect(result.ok ? result.value.parent.catalogId : null).toBe(PARENT);
    expect(result.ok ? result.value.subRooms[0]?.access : null).toBe('request');
    expect(sent(fetch)).toMatchObject({
      method: 'GET',
      url: `${BASE}/private-rooms/${SUB}/sub-rooms`,
    });
    expect(fetch).toHaveBeenCalledWith(
      expect.any(URL),
      expect.objectContaining({ credentials: 'include' }),
    );
  });

  it('“已归档”的名单带 archived=1，归档的那几项读得出来', async () => {
    const archived = {
      ...SUB_ROOM,
      joined: false,
      archivedAtUnixMs: 1_763_000_200_000,
      archivedReason: 'inactive',
      canRestore: true,
    };
    const { client, fetch } = clientAnswering(
      Response.json({
        parent: { catalogId: PARENT, name: 'Agent Room' },
        canCreate: false,
        categories: [],
        subRooms: [archived],
      }),
    );

    const result = await client.subRooms(PARENT, { archived: true });

    expect(result.ok ? result.value.subRooms[0]?.archivedReason : null).toBe('inactive');
    expect(sent(fetch).url).toBe(`${BASE}/private-rooms/${PARENT}/sub-rooms?archived=1`);
  });

  it('不认得的门不当真：回答读不懂时说 invalid_response', async () => {
    const { client } = clientAnswering(
      Response.json({
        parent: { catalogId: PARENT, name: 'Agent Room' },
        canCreate: true,
        categories: [],
        subRooms: [{ ...SUB_ROOM, access: 'password' }],
      }),
    );

    const result = await client.subRooms(PARENT);

    expect(result).toEqual({
      ok: false,
      error: { code: 'private_room.invalid_response', retryable: false },
    });
  });

  it('建子房间：新房间号是幂等键，不放进正文', async () => {
    const { client, fetch } = clientAnswering(Response.json(ROOM, { status: 201 }));

    const result = await client.create(PARENT, {
      catalogId: SUB,
      name: 'Release',
      description: 'Release checklist',
      access: 'request',
      categoryId: CATEGORY,
    });

    expect(result.ok ? result.value.catalogId : null).toBe(SUB);
    const request = sent(fetch);
    expect(request).toMatchObject({
      method: 'POST',
      url: `${BASE}/private-rooms/${PARENT}/sub-rooms`,
      body: {
        name: 'Release',
        description: 'Release checklist',
        access: 'request',
        categoryId: CATEGORY,
      },
    });
    expect(request.headers.get('Idempotency-Key')).toBe(SUB);
  });

  it('改：categoryId 写成 null 是挪回“未分类”', async () => {
    const { client, fetch } = clientAnswering(Response.json(ROOM));

    await client.revise(REF, { categoryId: null, hidden: true });

    expect(sent(fetch)).toMatchObject({
      method: 'PUT',
      url: `${BASE}/private-rooms/${PARENT}/sub-rooms/${SUB}`,
      body: { categoryId: null, hidden: true },
    });
  });

  it('申请：回答在等还是已经进来了；回答申请只说成没成', async () => {
    const asked = clientAnswering(
      Response.json({ catalogId: SUB, membership: 'pending', requestedAtUnixMs: 1 }),
    );
    const state = await asked.client.requestToJoin(REF, 'Can I help?');
    expect(state.ok ? state.value.membership : null).toBe('pending');
    expect(sent(asked.fetch)).toMatchObject({
      method: 'POST',
      url: `${BASE}/private-rooms/${PARENT}/sub-rooms/${SUB}/join-requests`,
      body: { message: 'Can I help?' },
    });

    const answered = clientAnswering(Response.json({ status: 'approved' }));
    const result = await answered.client.answerJoinRequest(REF, PERSON, 'approve');
    expect(result).toEqual({ ok: true, value: undefined });
    expect(sent(answered.fetch)).toMatchObject({
      method: 'PUT',
      url: `${BASE}/private-rooms/${PARENT}/sub-rooms/${SUB}/join-requests/${PERSON}`,
      body: { decision: 'approve' },
    });
  });

  it('在等的申请拿出名单本身', async () => {
    const request = {
      principalId: PERSON,
      displayName: 'Lin',
      message: '',
      requestedAtUnixMs: 1_763_000_000_000,
    };
    const { client } = clientAnswering(Response.json({ requests: [request] }));

    const result = await client.joinRequests(REF);

    expect(result).toEqual({ ok: true, value: [request] });
  });

  it.each([
    ['join', 'POST', '/membership'],
    ['leave', 'DELETE', '/membership'],
    ['archive', 'DELETE', ''],
    ['restore', 'POST', '/restore'],
    ['withdrawJoinRequest', 'DELETE', '/join-requests'],
  ] as const)('%s 走 %s …%s', async (action, method, suffix) => {
    const { client, fetch } = clientAnswering(
      action === 'join' ? Response.json(ROOM) : new Response(null, { status: 204 }),
    );

    const result = await client[action](REF);

    expect(result.ok).toBe(true);
    expect(sent(fetch)).toMatchObject({
      method,
      url: `${BASE}/private-rooms/${PARENT}/sub-rooms/${SUB}${suffix}`,
    });
  });

  it.each([
    ['admit', 'PUT', `/members/${PERSON}`],
    ['remove', 'DELETE', `/members/${PERSON}`],
    ['ban', 'POST', `/members/${PERSON}/ban`],
    ['unban', 'POST', `/members/${PERSON}/unban`],
    ['grantManager', 'PUT', `/managers/${PERSON}`],
    ['revokeManager', 'DELETE', `/managers/${PERSON}`],
  ] as const)('%s 走 %s …%s', async (action, method, suffix) => {
    const { client, fetch } = clientAnswering(new Response(null, { status: 204 }));

    const result = await client[action](REF, PERSON);

    expect(result.ok).toBe(true);
    expect(sent(fetch)).toMatchObject({
      method,
      url: `${BASE}/private-rooms/${PARENT}/sub-rooms/${SUB}${suffix}`,
    });
  });

  it('分类的几样都回改完以后的分类', async () => {
    const created = clientAnswering(Response.json({ categories: CATEGORIES }, { status: 201 }));
    expect(await created.client.createCategory(PARENT, 'Work')).toEqual({
      ok: true,
      value: CATEGORIES,
    });
    expect(sent(created.fetch)).toMatchObject({
      method: 'POST',
      url: `${BASE}/private-rooms/${PARENT}/sub-room-categories`,
      body: { name: 'Work' },
    });

    const moved = clientAnswering(Response.json({ categories: CATEGORIES }));
    await moved.client.reviseCategory(PARENT, CATEGORY, { position: 0 });
    expect(sent(moved.fetch)).toMatchObject({
      method: 'PUT',
      url: `${BASE}/private-rooms/${PARENT}/sub-room-categories/${CATEGORY}`,
      body: { position: 0 },
    });

    const deleted = clientAnswering(Response.json({ categories: [] }));
    expect(await deleted.client.deleteCategory(PARENT, CATEGORY)).toEqual({ ok: true, value: [] });
    expect(sent(deleted.fetch).method).toBe('DELETE');
  });

  it('房间的三项设置：改的时候不带 canChange', async () => {
    const policy = {
      creators: 'admins',
      agentsMayCreate: false,
      autoArchive: true,
      canChange: true,
    } as const;
    const { client, fetch } = clientAnswering(Response.json(policy));

    const result = await client.changePolicy(PARENT, {
      creators: 'admins',
      agentsMayCreate: false,
      autoArchive: true,
    });

    expect(result).toEqual({ ok: true, value: policy });
    expect(sent(fetch)).toMatchObject({
      method: 'PUT',
      url: `${BASE}/private-rooms/${PARENT}/sub-room-policy`,
      body: { creators: 'admins', agentsMayCreate: false, autoArchive: true },
    });
  });

  it('被拒时照服务器的错误码说', async () => {
    const { client } = clientAnswering(
      Response.json({ code: 'sub_room.target_banned', correlationId: 'corr-1' }, { status: 409 }),
    );

    const result = await client.admit(REF, PERSON);

    expect(result).toEqual({
      ok: false,
      error: { code: 'sub_room.target_banned', correlationId: 'corr-1', retryable: false },
    });
  });
});
