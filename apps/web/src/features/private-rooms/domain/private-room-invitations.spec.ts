import { describe, expect, it } from 'vitest';

import { parseInvitationList } from './private-room-invitations';

const first = '0198b601-77a2-7f41-b4f4-940f291951b8';
const second = '0198b601-77a2-7f41-b4f4-940f291951b9';

describe('新建房间时一起邀请的账户 ID', () => {
  it('空着就是不邀请', () => {
    expect(parseInvitationList('  \n ')).toEqual({ ok: true, principalIds: [] });
  });

  it('一行一个，也认空格、逗号和分号；重复的只算一次', () => {
    expect(parseInvitationList(`${first}\n${second}, ${first};`)).toEqual({
      ok: true,
      principalIds: [first, second],
    });
  });

  it('有一个不是账户 ID 就整体不通过', () => {
    expect(parseInvitationList(`${first}\nalice@example.com`)).toEqual({
      ok: false,
      reason: 'invalid',
    });
  });

  it('一次最多 50 个', () => {
    const many = Array.from(
      { length: 51 },
      (_, index) => `0198b601-77a2-7f41-b4f4-${index.toString(16).padStart(12, '0')}`,
    ).join('\n');
    expect(parseInvitationList(many)).toEqual({ ok: false, reason: 'too_many' });
  });
});
