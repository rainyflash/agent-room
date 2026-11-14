import { describe, expect, it } from 'vitest';

import { cursorOf, newMessagesAfter } from '@/features/conversation/domain/new-messages';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomMessageSignal } from '@/features/messages/domain/message';

const YOU = '@you:agent-room.test';
const ADA = '@ada:agent-room.test';

function say(id: string, at: number, by: string): RoomMessageSignal {
  const base = conversationFixture(id, { serverTimestamp: at });
  return {
    ...base,
    actor: {
      kind: 'human',
      principalId: `principal-${by}`,
      displayName: by,
      matrixUserId: by,
      provenance: 'human',
    },
  };
}

const timeline = [
  say('first', 10, ADA),
  say('mine', 20, YOU),
  say('second', 30, ADA),
  say('mine-again', 40, YOU),
  say('third', 50, ADA),
];
const at = (id: string) => {
  const message = timeline.find((entry) => entry.messageId === id);
  if (message === undefined) throw new Error('没有这条消息。');
  return cursorOf(message);
};

describe('上次看到的位置以后的新消息', () => {
  it('从别人说的第一条算起，你自己说的不算', () => {
    expect(newMessagesAfter(timeline, YOU, at('first'))).toEqual({ firstId: 'second', count: 2 });
    expect(newMessagesAfter(timeline, YOU, at('second'))).toEqual({ firstId: 'third', count: 1 });
  });

  it('都看过了，或者后面只有你自己说的：没有新消息', () => {
    expect(newMessagesAfter(timeline, YOU, at('third'))).toEqual({ firstId: null, count: 0 });
    expect(newMessagesAfter(timeline.slice(0, 4), YOU, at('second'))).toEqual({
      firstId: null,
      count: 0,
    });
  });

  it('没有“上次”可比时什么都不算新的', () => {
    expect(newMessagesAfter(timeline, YOU, null)).toEqual({ firstId: null, count: 0 });
  });

  it('还不知道你是谁时，后面的都算', () => {
    expect(newMessagesAfter(timeline, null, at('first'))).toEqual({ firstId: 'mine', count: 4 });
  });

  it('往前补出更早的消息不改变结果', () => {
    const older = [say('older', 1, ADA), ...timeline];
    expect(newMessagesAfter(older, YOU, at('first'))).toEqual({ firstId: 'second', count: 2 });
  });
});
