import { describe, expect, it } from 'vitest';

import {
  scopeConversation,
  unreadAddressed,
} from '@/features/conversation/domain/conversation-scope';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import { messageAttentionId, type RoomMessageSignal } from '@/features/messages/domain/message';

const YOU = '@you:agent-room.test';
const GRACE = '@grace:agent-room.test';
const ADA = '@ada:agent-room.test';
const ROOM = '!room:agent-room.test';

function say(
  id: string,
  at: number,
  by: string,
  extra: { readonly mentions?: readonly string[]; readonly replyTo?: string } = {},
): RoomMessageSignal {
  const base = conversationFixture(id, { roomId: ROOM, serverTimestamp: at });
  return {
    ...base,
    actor:
      by === ADA
        ? {
            kind: 'agent',
            agentId: 'agent-ada',
            instanceId: 'instance-ada',
            displayName: 'Ada',
            matrixUserId: ADA,
            provenance: 'autonomous_agent',
          }
        : {
            kind: 'human',
            principalId: `principal-${by}`,
            displayName: by,
            matrixUserId: by,
            provenance: 'human',
          },
    preview: {
      title: 'Conversation',
      summary: id,
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
      conversation: { text: id, mentions: extra.mentions ?? [] },
    },
    ...(extra.replyTo === undefined
      ? {}
      : { relation: { kind: 'reply' as const, targetMessageId: extra.replyTo } }),
  };
}

const timeline = [
  say('mine', 10, YOU, { mentions: [ADA] }),
  say('reply-to-me', 20, ADA, { replyTo: 'mine' }),
  say('chatter', 30, ADA),
  say('grace', 40, GRACE),
  say('mention-me', 50, GRACE, { mentions: [YOU] }),
  say('reply-to-grace', 60, ADA, { replyTo: 'grace' }),
];
const byId = new Map(timeline.map((message) => [message.messageId, message]));
const ids = (messages: readonly RoomMessageSignal[]): string[] =>
  messages.map((message) => message.messageId);

describe('只看：全部、人说的、提到我的', () => {
  it('全部原样；人说的只留人，连你自己说的', () => {
    expect(scopeConversation(timeline, 'all', YOU, byId)).toBe(timeline);
    expect(ids(scopeConversation(timeline, 'humans', YOU, byId))).toEqual([
      'mine',
      'grace',
      'mention-me',
    ]);
  });

  it('提到我的：别人点了你，或者回复你说的话；你自己说的不算', () => {
    expect(ids(scopeConversation(timeline, 'addressed', YOU, byId))).toEqual([
      'reply-to-me',
      'mention-me',
    ]);
    // 还不知道你是谁时一条也没有。
    expect(scopeConversation(timeline, 'addressed', null, byId)).toEqual([]);
  });
});

describe('提到我的有几条没读', () => {
  const none = new Set<string>();

  it('只数读到的位置以后的', () => {
    const cursor = (id: string) => {
      const message = byId.get(id);
      if (message === undefined) throw new Error('没有这条消息。');
      return { eventId: message.matrixEventId, timestamp: message.serverTimestamp };
    };
    expect(
      unreadAddressed(timeline, YOU, byId, { cursor: cursor('mine'), acknowledged: none }),
    ).toBe(2);
    expect(
      unreadAddressed(timeline, YOU, byId, { cursor: cursor('chatter'), acknowledged: none }),
    ).toBe(1);
    expect(
      unreadAddressed(timeline, YOU, byId, { cursor: cursor('mention-me'), acknowledged: none }),
    ).toBe(0);
  });

  it('收件箱里标过看过了的不数', () => {
    expect(
      unreadAddressed(timeline, YOU, byId, {
        cursor: { eventId: '$start', timestamp: 0 },
        acknowledged: new Set([messageAttentionId(ROOM, 'mention-me')]),
      }),
    ).toBe(1);
  });

  it('这间房还没有读到的位置、还不知道你是谁，都不数', () => {
    expect(unreadAddressed(timeline, YOU, byId, { cursor: undefined, acknowledged: none })).toBe(0);
    expect(
      unreadAddressed(timeline, null, byId, {
        cursor: { eventId: '$start', timestamp: 0 },
        acknowledged: none,
      }),
    ).toBe(0);
  });
});
