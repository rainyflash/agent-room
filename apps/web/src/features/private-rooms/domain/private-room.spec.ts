import { describe, expect, it } from 'vitest';

import { isPrivateRoomPrincipalId, joinedPrivateRoom, type PrivateRoom } from './private-room';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GUEST = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const ROOM = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: '',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    { permissions: { capabilities: ['view', 'speak'] }, principalId: GUEST, status: 'invited' },
  ],
  name: 'Sync probe',
  ownerPrincipalId: OWNER,
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 0,
} satisfies PrivateRoom;

describe('joinedPrivateRoom', () => {
  it('服务器记着你已经在里面：本机 Agent 替你建的房间，房主打开时要补上 Matrix 的加入', () => {
    expect(joinedPrivateRoom([ROOM], ROOM.catalogId, ROOM.matrixRoomId, OWNER)).toBe(ROOM);
    // 归档了的照样补：加入以后才看得到之前的消息。
    const archived = { ...ROOM, status: 'archived' } satisfies PrivateRoom;
    expect(joinedPrivateRoom([archived], ROOM.catalogId, ROOM.matrixRoomId, OWNER)).toBe(archived);
  });

  it('还没答复的邀请、不在房间里的人、对不上的房间都不替你加入', () => {
    expect(joinedPrivateRoom([ROOM], ROOM.catalogId, ROOM.matrixRoomId, GUEST)).toBeNull();
    expect(
      joinedPrivateRoom(
        [ROOM],
        ROOM.catalogId,
        ROOM.matrixRoomId,
        '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
      ),
    ).toBeNull();
    expect(joinedPrivateRoom([ROOM], ROOM.catalogId, '!other:matrix.test', OWNER)).toBeNull();
    expect(
      joinedPrivateRoom([ROOM], '0198b601-77a1-7bb8-83eb-a8fe68c97e40', ROOM.matrixRoomId, OWNER),
    ).toBeNull();
    expect(joinedPrivateRoom([], ROOM.catalogId, ROOM.matrixRoomId, OWNER)).toBeNull();
  });
});

describe('isPrivateRoomPrincipalId', () => {
  it('接受主体 ID 并容忍首尾空白', () => {
    expect(isPrivateRoomPrincipalId('01a04765-56e7-7673-8d7c-5d301e9020af')).toBe(true);
    expect(isPrivateRoomPrincipalId('  01a04765-56e7-7673-8d7c-5d301e9020af  ')).toBe(true);
  });

  it('拒绝昵称、邮箱和 Matrix 身份', () => {
    // 产品不提供按昵称或邮箱检索账号，这些输入必须在发出请求之前就被挡住。
    for (const value of [
      '',
      'Ziyang Zhang',
      'owner@example.com',
      '@owner:agentroom.chat',
      '01a04765-56e7-7673-8d7c',
      '01a04765-56e7-7673-8d7c-5d301e9020af-extra',
    ]) {
      expect(isPrivateRoomPrincipalId(value)).toBe(false);
    }
  });
});
