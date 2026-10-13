import { describe, expect, it } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { groupMyPrivateRooms } from '@/features/room-directory/domain/my-private-rooms';

const me = '0198b601-77a2-7f41-b4f4-940f29195000';

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
    ownerPrincipalId: '0198b601-77a2-7f41-b4f4-940f29195999',
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
