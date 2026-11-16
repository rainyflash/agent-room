import { describe, expect, it } from 'vitest';

import {
  characterCount,
  normalizeRoomNotice,
  parseRoomNotice,
  parseRoomNoticeChange,
  roomNoticeProblem,
  briefSeenKey,
} from '@/features/room-notice/domain/room-notice';

describe('房间须知的规则', () => {
  it('按 Unicode 字符数算，和服务器一样：2000 个可以，多一个不行', () => {
    expect(characterCount('公告😀')).toBe(3);
    expect(roomNoticeProblem('公'.repeat(2_000))).toBeNull();
    expect(roomNoticeProblem('😀'.repeat(2_001))).toBe('too_long');
    expect(roomNoticeProblem(`  ${'公'.repeat(2_000)}  `)).toBeNull();
  });

  it('能换行、能有制表符；回车换成换行再看，别的控制字符不收', () => {
    expect(normalizeRoomNotice('  第一行\r\n第二行\r第三行 ')).toBe('第一行\n第二行\n第三行');
    expect(roomNoticeProblem('第一行\r\n\t第二行')).toBeNull();
    expect(roomNoticeProblem('响铃\u0007')).toBe('control_character');
    // 首尾的空白（比如换页）先去掉，夹在中间的才算。
    expect(roomNoticeProblem('换\u000C页')).toBe('control_character');
    expect(roomNoticeProblem('C1\u0085控制字符')).toBe('control_character');
  });

  it('房间状态里的公告：时间换成毫秒，正文去掉首尾空白；空的、类型不对的当没有', () => {
    const state = {
      eventType: 'io.github.rainyflash.agentroom.room.notice.v1',
      schemaVersion: '1.0',
      text: ' 只在被点名时说话。 ',
      updatedAt: '2026-11-03T08:00:00.123Z',
      updatedBy: '@owner:matrix.test',
      future: 'ignored',
    };
    expect(parseRoomNotice(state)).toEqual({
      text: '只在被点名时说话。',
      updatedAtUnixMs: Date.parse('2026-11-03T08:00:00.123Z'),
      updatedBy: '@owner:matrix.test',
    });
    expect(parseRoomNotice({ ...state, text: '' })).toBeUndefined();
    expect(
      parseRoomNotice({ ...state, eventType: 'io.github.rainyflash.agentroom.room.listing.v1' }),
    ).toBeUndefined();
    expect(parseRoomNotice({ ...state, updatedAt: 'yesterday' })).toBeUndefined();
    expect(parseRoomNotice(null)).toBeUndefined();
  });

  it('一次改动：有正文是更新，空的是撤掉；读不出来、超长的不算', () => {
    const state = {
      eventType: 'io.github.rainyflash.agentroom.room.notice.v1',
      schemaVersion: '1.0',
      text: '只在被点名时说话。',
      updatedAt: '2026-11-03T08:00:00.123Z',
      updatedBy: '@owner:matrix.test',
    };
    expect(parseRoomNoticeChange(state)).toEqual({
      kind: 'updated',
      text: '只在被点名时说话。',
      updatedBy: '@owner:matrix.test',
    });
    expect(parseRoomNoticeChange({ ...state, text: '  ' })).toEqual({
      kind: 'removed',
      text: '',
      updatedBy: '@owner:matrix.test',
    });
    expect(parseRoomNoticeChange({ ...state, text: '公'.repeat(2_001) })).toBeUndefined();
    expect(parseRoomNoticeChange({ ...state, updatedBy: 'owner' })).toBeUndefined();
    // 撤回（redact）以后内容是空的。
    expect(parseRoomNoticeChange({})).toBeUndefined();
  });

  it('看过哪一版记在个人工作区的键：brief 加房间 ID，太长的放不下就不记', () => {
    expect(briefSeenKey('!club:matrix.test')).toBe('brief:!club:matrix.test');
    expect(briefSeenKey(`!${'a'.repeat(503)}:m`)).toBe(`brief:!${'a'.repeat(503)}:m`);
    expect(briefSeenKey(`!${'a'.repeat(504)}:m`)).toBeNull();
  });
});
