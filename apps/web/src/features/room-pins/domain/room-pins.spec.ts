import { describe, expect, it } from 'vitest';

import {
  parseRoomPins,
  parseRoomPinsChange,
  pinsSeenKey,
  roomPinsField,
} from '@/features/room-pins/domain/room-pins';

const GRACE = '@grace:matrix.test';

const state = (overrides: Record<string, unknown> = {}, field: Record<string, unknown> = {}) => ({
  pinned: ['$b', '$a'],
  [roomPinsField]: {
    schemaVersion: '1.0',
    entries: [
      { eventId: '$b', pinnedBy: GRACE, pinnedAt: '2026-11-16T08:00:00.000Z' },
      { eventId: '$a', pinnedBy: '@mina:matrix.test', pinnedAt: '2026-11-15T08:00:00.000Z' },
    ],
    updatedBy: GRACE,
    updatedAt: '2026-11-16T08:00:00.000Z',
    change: { kind: 'pinned', eventId: '$b' },
    ...field,
  },
  ...overrides,
});

describe('房间状态里的置顶', () => {
  it('照我们的字段写谁、什么时候置顶的，时间换成毫秒，新的在最前面', () => {
    expect(parseRoomPins(state())).toEqual({
      entries: [
        { eventId: '$b', pinnedBy: GRACE, pinnedAtUnixMs: Date.parse('2026-11-16T08:00:00.000Z') },
        {
          eventId: '$a',
          pinnedBy: '@mina:matrix.test',
          pinnedAtUnixMs: Date.parse('2026-11-15T08:00:00.000Z'),
        },
      ],
      updatedAtUnixMs: Date.parse('2026-11-16T08:00:00.000Z'),
      updatedBy: GRACE,
    });
  });

  it('标准的列表说了算：已经取消的不写；我们的字段里重复的只算一次', () => {
    const pins = parseRoomPins(
      state(
        { pinned: ['$b'] },
        {
          entries: [
            { eventId: '$b', pinnedBy: GRACE, pinnedAt: '2026-11-16T08:00:00.000Z' },
            { eventId: '$b', pinnedBy: GRACE, pinnedAt: '2026-11-14T08:00:00.000Z' },
            { eventId: '$a', pinnedBy: GRACE, pinnedAt: '2026-11-15T08:00:00.000Z' },
          ],
        },
      ),
    );
    expect(pins?.entries.map((pin) => pin.eventId)).toEqual(['$b']);
    expect(pins?.entries[0]?.pinnedAtUnixMs).toBe(Date.parse('2026-11-16T08:00:00.000Z'));
  });

  it('一条都没有的、没有我们的字段的（不是我们写的）、读不出来的都当没有置顶', () => {
    expect(parseRoomPins(state({ pinned: [] }))).toBeUndefined();
    expect(parseRoomPins({ pinned: ['$b'] })).toBeUndefined();
    expect(parseRoomPins(state({}, { updatedAt: 'yesterday' }))).toBeUndefined();
    expect(parseRoomPins(state({}, { change: { kind: 'moved', eventId: '$b' } }))).toBeUndefined();
    expect(parseRoomPins(undefined)).toBeUndefined();
  });

  it('一次改动：置顶还是取消、哪一句、谁改的', () => {
    expect(parseRoomPinsChange(state())).toEqual({
      kind: 'pinned',
      eventId: '$b',
      updatedBy: GRACE,
    });
    expect(
      parseRoomPinsChange(
        state({ pinned: ['$a'] }, { change: { kind: 'unpinned', eventId: '$b' } }),
      ),
    ).toEqual({ kind: 'unpinned', eventId: '$b', updatedBy: GRACE });
    expect(parseRoomPinsChange({ pinned: [] })).toBeUndefined();
  });

  it('看过哪一次记在 pins:<房间>，放不下的房间 ID 不记', () => {
    expect(pinsSeenKey('!club:matrix.test')).toBe('pins:!club:matrix.test');
    expect(pinsSeenKey(`!${'x'.repeat(600)}:matrix.test`)).toBeNull();
  });
});
