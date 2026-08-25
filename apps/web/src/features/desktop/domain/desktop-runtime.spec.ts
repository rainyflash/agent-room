import { describe, expect, it } from 'vitest';

import {
  desktopDeepLinkSchema,
  parseLobbyDeepLinkRoute,
} from '@/features/desktop/domain/desktop-runtime';

describe('桌面深链领域模型', () => {
  it('只把已实现的大厅路径投影为类型化导航', () => {
    expect(parseLobbyDeepLinkRoute('/lobby/public')).toEqual({
      catalogId: 'public',
      kind: 'catalog',
    });
    expect(parseLobbyDeepLinkRoute('/lobby/public/instance/!room:example.org')).toEqual({
      catalogId: 'public',
      kind: 'instance',
      roomId: '!room:example.org',
    });
  });

  it('拒绝开放跳转、未实现页面和额外路径段', () => {
    expect(parseLobbyDeepLinkRoute('//evil.example')).toBeNull();
    expect(parseLobbyDeepLinkRoute('/handoffs/0199')).toBeNull();
    expect(parseLobbyDeepLinkRoute('/lobby/public/instance/room/extra')).toBeNull();
    expect(
      desktopDeepLinkSchema.safeParse({ kind: 'lobby', route: '//evil.example' }).success,
    ).toBe(false);
  });
});
