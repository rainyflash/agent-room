import { describe, expect, it } from 'vitest';

import { shownMemberName } from './member-names';

const id = '@user-3412b8ab0ef3cf6a658cbdcf29595500:matrix.agent-room.localhost';

describe('成员在 Matrix 上的名字', () => {
  it('设了昵称就写昵称，去掉首尾空白', () => {
    expect(shownMemberName(id, '  小王 ')).toBe('小王');
  });

  it('名字是邮箱时只留 @ 前面那段，和服务器给成员名字的规则一样', () => {
    expect(shownMemberName(id, 'wang@example.com')).toBe('wang');
  });

  it.each([
    ['没设名字，SDK 给的是用户 ID', id],
    ['名字就是用户名', 'user-3412b8ab0ef3cf6a658cbdcf29595500'],
    ['只有空白', '   '],
    ['截完什么也不剩', '@example.com'],
  ])('%s时认不出名字', (_, displayName) => {
    expect(shownMemberName(id, displayName)).toBeNull();
  });
});
