import { describe, expect, it } from 'vitest';

import { parseRoomOwnerHandover } from '@/features/private-rooms/domain/owner-handover';

describe('房间换了房主的那条状态', () => {
  const state = {
    schemaVersion: '1.0',
    eventType: 'io.github.rainyflash.agentroom.room.owner_handover.v1',
    owner: '@admin:matrix.agent-room.localhost',
    formerOwner: '@owner:matrix.agent-room.localhost',
  };

  it('读出接手的人', () => {
    expect(parseRoomOwnerHandover(state)).toEqual({ owner: '@admin:matrix.agent-room.localhost' });
  });

  it('类型不对、少了谁、ID 不像 Matrix 用户的都不认', () => {
    expect(
      parseRoomOwnerHandover({
        ...state,
        eventType: 'io.github.rainyflash.agentroom.room.notice.v1',
      }),
    ).toBeUndefined();
    expect(parseRoomOwnerHandover({ ...state, owner: 'admin' })).toBeUndefined();
    expect(parseRoomOwnerHandover({ ...state, formerOwner: undefined })).toBeUndefined();
    expect(parseRoomOwnerHandover({})).toBeUndefined();
  });
});
