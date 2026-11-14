import { describe, expect, it } from 'vitest';

import {
  defaultRoomPageLayout,
  rememberRoomPageLayout,
  rememberedRoomPageLayout,
  resolveRoomPageLayout,
} from './room-page-layout';

function memoryStorage(initial: Readonly<Record<string, string>> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}

describe('房间页的版面', () => {
  it('地址里写了的优先，其次是这台设备记着的，最后是默认', () => {
    expect(resolveRoomPageLayout('conversation', 'scene')).toBe('conversation');
    expect(resolveRoomPageLayout(undefined, 'conversation')).toBe('conversation');
    expect(resolveRoomPageLayout(undefined, null)).toBe(defaultRoomPageLayout);
  });

  it('记下选择，下次读得回来', () => {
    const storage = memoryStorage();
    expect(rememberedRoomPageLayout(storage)).toBeNull();
    rememberRoomPageLayout('conversation', storage);
    expect(rememberedRoomPageLayout(storage)).toBe('conversation');
  });

  it('不认识的值和读写不了的存储都当没选过', () => {
    expect(
      rememberedRoomPageLayout(memoryStorage({ 'agent-room.room-page-layout': 'list' })),
    ).toBeNull();
    const broken = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(rememberedRoomPageLayout(broken)).toBeNull();
    expect(() => {
      rememberRoomPageLayout('scene', broken);
    }).not.toThrow();
    expect(rememberedRoomPageLayout(null)).toBeNull();
  });
});
