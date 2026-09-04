import { describe, expect, it } from 'vitest';

import {
  selectPreferredPublicRoom,
  type PublicRoomSummary,
} from '@/features/room-directory/domain/public-room-directory';

const englishRoom = room('0198b601-77a2-7f41-b4f4-940f291951b8', 'English', 'en');
const chineseRoom = room('0198b601-77a3-74f1-b4f4-940f291951b9', '中文', 'zh-CN');
const rooms: readonly PublicRoomSummary[] = [englishRoom, chineseRoom];

describe('公共房间目录领域策略', () => {
  it('优先选择精确语言，再选择同语系，最后保留服务端顺序', () => {
    expect(selectPreferredPublicRoom(rooms, 'zh-CN')?.name).toBe('中文');
    expect(selectPreferredPublicRoom(rooms, 'zh-TW')?.name).toBe('中文');
    expect(selectPreferredPublicRoom(rooms, 'fr-FR')?.name).toBe('English');
  });

  it('空目录不伪造房间', () => {
    expect(selectPreferredPublicRoom([], 'en')).toBeNull();
  });
});

function room(catalogId: string, name: string, language: string): PublicRoomSummary {
  return {
    activeInstanceCount: 1,
    catalogId,
    description: '',
    language,
    name,
    onlineAgentCount: 1,
    slug: name.toLowerCase(),
  };
}
