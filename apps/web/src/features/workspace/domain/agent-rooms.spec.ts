import { describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { err, ok, type Result } from '@/shared/result';

import type { AgentDirectoryFailure } from './agent-directory';
import { bringCandidates, bringWhenReady } from './agent-rooms';

const ME = '0198b601-77a3-74f1-b4f4-940f291951b9';

describe('带进房间', () => {
  it('列出你能带它进的房间：私人房间要你加入、能发言，它已经在的不列', () => {
    const candidates = bringCandidates({
      current: [
        {
          archived: false,
          catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e60',
          kind: 'private_room',
          name: 'Weekly review',
        },
      ],
      principalId: ME,
      privateRooms: [
        privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e60', 'Weekly review', ['view', 'speak']),
        privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e61', 'Design crit', [
          'view',
          'speak',
          'automate',
        ]),
        privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e62', 'Board notes', ['view', 'speak']),
        privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e63', 'Read only', ['view']),
        {
          ...privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e64', 'Invited', ['view', 'speak']),
          members: [
            {
              permissions: { capabilities: ['view', 'speak'] },
              principalId: ME,
              status: 'invited',
            },
          ],
        },
        {
          ...privateRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e65', 'Closed', ['view', 'speak']),
          status: 'archived',
        },
      ],
      publicRooms: [publicRoom('0198b601-77a7-7bb8-83eb-a8fe68c97e66', 'Builders Exchange')],
    });

    expect(candidates).toEqual([
      {
        catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e61',
        kind: 'private_room',
        name: 'Design crit',
        viewOnly: false,
      },
      {
        catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e62',
        kind: 'private_room',
        name: 'Board notes',
        viewOnly: true,
      },
      {
        catalogId: '0198b601-77a7-7bb8-83eb-a8fe68c97e66',
        kind: 'public_lobby',
        name: 'Builders Exchange',
        viewOnly: false,
      },
    ]);
  });

  it('房间正在准备时照服务器说的等一会儿再试，最多试三次', async () => {
    const busy = err({ code: 'agent_room.busy', retryable: true, retryAfterSeconds: 2 });
    const bring = vi
      .fn<() => Promise<Result<number, AgentDirectoryFailure>>>()
      .mockResolvedValueOnce(busy)
      .mockResolvedValueOnce(ok(1));
    const sleep = vi.fn(() => Promise.resolve());

    expect(await bringWhenReady(bring, sleep)).toEqual(ok(1));
    expect(sleep).toHaveBeenCalledWith(2_000);

    bring.mockReset().mockResolvedValue(busy);
    sleep.mockClear();
    expect(await bringWhenReady(bring, sleep)).toEqual(busy);
    expect(bring).toHaveBeenCalledTimes(3);
    expect(sleep).toHaveBeenCalledTimes(2);
  });

  it('别的失败不重试', async () => {
    const forbidden = err({ code: 'lobby.unauthorized', retryable: false });
    const bring = vi.fn(() => Promise.resolve(forbidden));
    const sleep = vi.fn(() => Promise.resolve());

    expect(await bringWhenReady(bring, sleep)).toEqual(forbidden);
    expect(bring).toHaveBeenCalledOnce();
    expect(sleep).not.toHaveBeenCalled();
  });
});

function privateRoom(
  catalogId: string,
  name: string,
  capabilities: PrivateRoom['members'][number]['permissions']['capabilities'],
): PrivateRoom {
  return {
    catalogId,
    description: '',
    matrixRoomId: '!room:matrix.test',
    members: [{ permissions: { capabilities }, principalId: ME, status: 'joined' }],
    name,
    ownerPrincipalId: ME,
    retentionDays: null,
    roomInstanceId: catalogId,
    status: 'active',
    version: 1,
  };
}

function publicRoom(catalogId: string, name: string): PublicRoomSummary {
  return {
    activeInstanceCount: 1,
    catalogId,
    description: '',
    language: 'en',
    name,
    onlineAgentCount: 0,
    slug: null,
  };
}
