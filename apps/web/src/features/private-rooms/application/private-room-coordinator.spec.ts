import { describe, expect, it, vi } from 'vitest';

import { PrivateRoomCoordinator } from './private-room-coordinator';
import type {
  PrivateRoom,
  PrivateRoomGateway,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';
import { err, ok } from '@/shared/result';

const ROOM = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: '',
  matrixRoomId: '!private:matrix.test',
  members: [],
  name: 'Architecture room',
  ownerPrincipalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 0,
} satisfies PrivateRoom;

describe('PrivateRoomCoordinator', () => {
  it('真实 Matrix 加入成功前绝不把新房间交给界面', async () => {
    const rooms = gateway();
    const matrix = matrixGateway();
    rooms.spies.create.mockResolvedValue(ok(ROOM));
    matrix.spies.join.mockResolvedValue(
      err({ code: 'private_room.matrix_join_failed', retryable: true }),
    );
    const coordinator = new PrivateRoomCoordinator(rooms.value, matrix.value);

    const result = await coordinator.createAndJoin(ROOM.catalogId, {
      description: '',
      invitations: [],
      name: ROOM.name,
    });

    expect(result).toEqual({
      error: { code: 'private_room.matrix_join_failed', retryable: true },
      ok: false,
    });
    expect(matrix.spies.join).toHaveBeenCalledWith(ROOM.matrixRoomId);
  });

  it('接受邀请严格按 Matrix 加入后产品确认的顺序执行', async () => {
    const calls: string[] = [];
    const rooms = gateway();
    const matrix = matrixGateway();
    matrix.spies.join.mockImplementation(() => {
      calls.push('matrix.join');
      return Promise.resolve(ok(undefined));
    });
    rooms.spies.accept.mockImplementation(() => {
      calls.push('control.accept');
      return Promise.resolve(ok(ROOM));
    });

    const result = await new PrivateRoomCoordinator(rooms.value, matrix.value).accept(ROOM);

    expect(result).toEqual(ok(ROOM));
    expect(calls).toEqual(['matrix.join', 'control.accept']);
  });
});

function gateway() {
  const unavailable = vi
    .fn()
    .mockResolvedValue(err({ code: 'private_room.test_unavailable', retryable: false }));
  const accept = vi
    .fn<PrivateRoomGateway['accept']>()
    .mockResolvedValue(err({ code: 'private_room.test_unavailable', retryable: false }));
  const create = vi
    .fn<PrivateRoomGateway['create']>()
    .mockResolvedValue(err({ code: 'private_room.test_unavailable', retryable: false }));
  const value: PrivateRoomGateway = {
    accept,
    archive: vi.fn(unavailable),
    ban: vi.fn(unavailable),
    create,
    decline: vi.fn(unavailable),
    inspect: vi.fn(unavailable),
    invite: vi.fn(unavailable),
    leave: vi.fn(unavailable),
    list: vi.fn(unavailable),
    remove: vi.fn(unavailable),
    transferOwnership: vi.fn(unavailable),
    updatePermissions: vi.fn(unavailable),
  };
  return { spies: { accept, create }, value };
}

function matrixGateway() {
  const join = vi.fn().mockResolvedValue(ok(undefined));
  const value: PrivateRoomMatrixGateway = {
    join,
    leave: vi.fn().mockResolvedValue(ok(undefined)),
  };
  return { spies: { join }, value };
}
