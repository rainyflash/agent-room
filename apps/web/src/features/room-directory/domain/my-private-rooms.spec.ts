import { describe, expect, it } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import {
  groupMyPrivateRooms,
  roomOwnerOf,
  splitByOwner,
} from '@/features/room-directory/domain/my-private-rooms';

const me = '0198b601-77a2-7f41-b4f4-940f29195000';
const someoneElse = '0198b601-77a2-7f41-b4f4-940f29195999';

function room(
  name: string,
  status: PrivateRoom['status'],
  membership: PrivateRoom['members'][number]['status'] | null,
  suffix: string,
): PrivateRoom {
  return {
    catalogId: `0198b601-77a2-7f41-b4f4-940f291951${suffix}`,
    description: '',
    matrixRoomId: `!${suffix}:matrix.test`,
    members:
      membership === null
        ? []
        : [
            {
              permissions: { capabilities: ['view', 'speak'] },
              principalId: me,
              status: membership,
            },
          ],
    name,
    ownerPrincipalId: someoneElse,
    retentionDays: null,
    roomInstanceId: `0198b601-77a2-7f41-b4f4-940f291952${suffix}`,
    status,
    version: 1,
  };
}

describe('你的私人房间按身份分开', () => {
  it('还在用的分成受邀和已加入；归档的单独一组，只算你加入过的', () => {
    const invited = room('Research lab', 'active', 'invited', '01');
    const joined = room('Design studio', 'active', 'joined', '02');
    const archived = room('Spring launch', 'archived', 'joined', '03');
    const grouped = groupMyPrivateRooms(
      [
        invited,
        joined,
        archived,
        room('Declined', 'active', 'declined', '04'),
        room('Left before archive', 'archived', 'removed', '05'),
        room('Invited then archived', 'archived', 'invited', '06'),
        room('Not mine', 'archived', null, '07'),
      ],
      me,
    );

    expect(grouped).toEqual({ invitations: [invited], joined: [joined], archived: [archived] });
  });

  it('还不知道是谁时一间也不算', () => {
    const grouped = groupMyPrivateRooms(
      [room('Design studio', 'active', 'joined', '02'), room('Old', 'archived', 'joined', '03')],
      undefined,
    );

    expect(grouped).toEqual({ invitations: [], joined: [], archived: [] });
  });
});

describe('已加入的房间按房主分开', () => {
  it('房主是你的归“你建的”（你的 Agent 替你建的房主也是你），别的归“你加入的”', () => {
    const mine = { ...room('Design studio', 'active', 'joined', '02'), ownerPrincipalId: me };
    const theirs = room('Partner workshop', 'active', 'joined', '08');

    expect(splitByOwner([theirs, mine], me)).toEqual({ created: [mine], others: [theirs] });
  });

  it('还不知道是谁时都算别人的', () => {
    const mine = { ...room('Design studio', 'active', 'joined', '02'), ownerPrincipalId: me };

    expect(splitByOwner([mine], undefined)).toEqual({ created: [], others: [mine] });
  });
});

describe('别人房间的房主', () => {
  const theirs = room('Partner workshop', 'active', 'joined', '08');
  const byPerson = { createdByAgent: null, ownerIsNetworkAgent: false };
  const people = [
    { displayName: '  Mia ', principalId: someoneElse },
    { displayName: 'Me', principalId: me },
  ];

  it('房主是人：用成员名单里他的昵称', () => {
    expect(roomOwnerOf(theirs, byPerson, people)).toEqual({ kind: 'person', name: 'Mia' });
  });

  it('房主是网络 Agent：写建房的 Agent 的名字，不看成员名单；名字查不到时为空', () => {
    const agent = { agentId: '0198b601-77a2-7f41-b4f4-940f29196001', displayName: 'Atlas' };

    expect(
      roomOwnerOf(theirs, { createdByAgent: agent, ownerIsNetworkAgent: true }, undefined),
    ).toEqual({ kind: 'network_agent', name: 'Atlas' });
    expect(
      roomOwnerOf(
        theirs,
        { createdByAgent: { ...agent, displayName: null }, ownerIsNetworkAgent: true },
        people,
      ),
    ).toEqual({ kind: 'network_agent', name: null });
  });

  it('还不知道是谁建的、名单没问到或名单里没有房主时先不写', () => {
    expect(roomOwnerOf(theirs, undefined, people)).toBeNull();
    expect(roomOwnerOf(theirs, byPerson, undefined)).toBeNull();
    expect(roomOwnerOf(theirs, byPerson, [{ displayName: 'Me', principalId: me }])).toBeNull();
    expect(
      roomOwnerOf(theirs, byPerson, [{ displayName: '   ', principalId: someoneElse }]),
    ).toBeNull();
  });
});
