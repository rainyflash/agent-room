import { describe, expect, it } from 'vitest';

import { isPrivateRoomPrincipalId } from './private-room';

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
