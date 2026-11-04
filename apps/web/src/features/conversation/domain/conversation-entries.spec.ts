import { describe, expect, it } from 'vitest';

import { withNoticeChanges } from '@/features/conversation/domain/conversation-entries';
import type { RoomMessageSignal, RoomNoticeChange } from '@/features/messages/domain/message';

function message(id: string, serverTimestamp: number): RoomMessageSignal {
  return { messageId: id, serverTimestamp } as RoomMessageSignal;
}

function change(id: string, serverTimestamp: number): RoomNoticeChange {
  return { kind: 'updated', matrixEventId: id, serverTimestamp, updatedBy: '@owner:matrix.test' };
}

function ids(entries: ReturnType<typeof withNoticeChanges>): string[] {
  return entries.map((entry) =>
    entry.kind === 'message' ? entry.message.messageId : entry.change.matrixEventId,
  );
}

describe('对话里夹着公告的改动', () => {
  it('按时间夹进消息里，同一刻的先消息后改动，比最后一条还晚的放在最后', () => {
    const messages = [message('a', 10), message('b', 20), message('c', 30)];
    const changes = [change('$before', 5), change('$tie', 20), change('$late', 40)];
    expect(ids(withNoticeChanges(messages, changes))).toEqual([
      '$before',
      'a',
      'b',
      '$tie',
      'c',
      '$late',
    ]);
  });

  it('没有消息时只有改动，没有改动时只有消息', () => {
    expect(ids(withNoticeChanges([], [change('$only', 5)]))).toEqual(['$only']);
    expect(ids(withNoticeChanges([message('a', 10)], []))).toEqual(['a']);
  });
});
