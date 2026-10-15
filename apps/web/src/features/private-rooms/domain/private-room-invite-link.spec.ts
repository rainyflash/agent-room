import { describe, expect, it } from 'vitest';

import {
  parseInviteSecret,
  privateRoomInviteLinkSchema,
  privateRoomInvitePreviewSchema,
} from './private-room-invite-link';

const SECRET = '0123456789abcdefghjkmnpqrs';

describe('邀请链接', () => {
  it('秘密是 26 个小写 Crockford Base32 字符，大写和空白照收', () => {
    expect(parseInviteSecret(SECRET)).toBe(SECRET);
    expect(parseInviteSecret(`  ${SECRET.toUpperCase()}\n`)).toBe(SECRET);
    // Crockford 不用 i、l、o、u。
    expect(parseInviteSecret(`${SECRET.slice(0, 25)}i`)).toBeNull();
    expect(parseInviteSecret(SECRET.slice(1))).toBeNull();
    expect(parseInviteSecret('')).toBeNull();
  });

  it('服务器的回答严格校验：多一个字段、链接不是 http(s) 都不认', () => {
    const link = {
      createdAtUnixMs: 1,
      expiresAtUnixMs: 2,
      permissions: { capabilities: ['view', 'speak', 'automate'] },
      secret: SECRET,
      url: `https://agentroom.chat/join#${SECRET}`,
    };
    expect(privateRoomInviteLinkSchema.safeParse(link).success).toBe(true);
    expect(privateRoomInviteLinkSchema.safeParse({ ...link, extra: true }).success).toBe(false);
    expect(
      privateRoomInviteLinkSchema.safeParse({ ...link, url: `javascript:alert(1)//#${SECRET}` })
        .success,
    ).toBe(false);
    expect(
      privateRoomInvitePreviewSchema.safeParse({
        catalogId: '0198b601-77a4-7f41-b4f4-940f291951c0',
        expiresAtUnixMs: 2,
        inviterDisplayName: 'Ada',
        membership: 'left',
        permissions: { capabilities: ['view'] },
        roomName: 'Weekly',
      }).success,
    ).toBe(false);
  });
});
