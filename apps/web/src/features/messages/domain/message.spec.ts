import { describe, expect, it } from 'vitest';

import {
  addressesAccount,
  mentionsAccount,
  messageAttentionId,
  type RoomMessageSignal,
} from './message';

const me = '@me:agent-room.test';

function message(
  sender: string,
  mentions: readonly string[],
  mentionsEveryone = false,
): RoomMessageSignal {
  return {
    actor: {
      agentId: '01990d9e-8400-7000-8000-000000000005',
      displayName: 'Ada',
      instanceId: '01990d9e-8400-7000-8000-000000000006',
      kind: 'agent',
      matrixUserId: sender,
      provenance: 'autonomous_agent',
    },
    content: null,
    edited: false,
    endToEndEncrypted: mentionsEveryone,
    lifecycle: 'active',
    matrixEventId: '$event',
    messageId: '01990d9e-8400-7000-8000-000000000004',
    preview: {
      contentType: 'text/plain',
      conversation: { mentions, text: 'Done.' },
      ...(mentionsEveryone ? { mentionsEveryone: true as const } : {}),
      riskFlags: [],
      sensitivity: 'normal',
      summary: 'Done.',
      title: 'Done.',
    },
    roomId: '!room:agent-room.test',
    serverTimestamp: 1_000,
    signatureStatus: 'matrix_sender_matched',
  };
}

describe('这条消息是不是在找你', () => {
  it('别人点了你的名字，或者 @了所有人，都算', () => {
    expect(mentionsAccount(message('@ada:agent-room.test', [me]), me)).toBe(true);
    expect(mentionsAccount(message('@ada:agent-room.test', [], true), me)).toBe(true);
  });

  it('点的是别人、谁也没点，不算', () => {
    expect(mentionsAccount(message('@ada:agent-room.test', ['@lin:agent-room.test']), me)).toBe(
      false,
    );
    expect(mentionsAccount(message('@ada:agent-room.test', []), me)).toBe(false);
  });

  it('自己发的不算，哪怕点了自己或 @了所有人', () => {
    expect(mentionsAccount(message(me, [me]), me)).toBe(false);
    expect(mentionsAccount(message(me, [], true), me)).toBe(false);
  });

  it('没有预览的消息不算', () => {
    expect(mentionsAccount({ ...message('@ada:agent-room.test', [me]), preview: null }, me)).toBe(
      false,
    );
  });
});

describe('这条消息是不是冲着你来的', () => {
  const mine = { ...message(me, []), messageId: 'mine' };
  const theirs = { ...message('@lin:agent-room.test', []), messageId: 'theirs' };
  const byId = new Map([
    [mine.messageId, mine],
    [theirs.messageId, theirs],
  ]);
  const reply = (sender: string, target: string): RoomMessageSignal => ({
    ...message(sender, []),
    relation: { kind: 'reply', targetMessageId: target },
  });

  it('点了你是 mention，回复你说的话是 reply；两样都有时算 mention', () => {
    expect(addressesAccount(message('@ada:agent-room.test', [me]), byId, me)).toBe('mention');
    expect(addressesAccount(reply('@ada:agent-room.test', 'mine'), byId, me)).toBe('reply');
    expect(
      addressesAccount(
        { ...reply('@ada:agent-room.test', 'mine'), preview: message(me, [me]).preview },
        byId,
        me,
      ),
    ).toBe('mention');
  });

  it('回复的是别人、被回复的那句没加载到、自己回复自己，都不算', () => {
    expect(addressesAccount(reply('@ada:agent-room.test', 'theirs'), byId, me)).toBeNull();
    expect(addressesAccount(reply('@ada:agent-room.test', 'not-loaded'), byId, me)).toBeNull();
    expect(addressesAccount(reply(me, 'mine'), byId, me)).toBeNull();
    expect(addressesAccount(message('@ada:agent-room.test', []), byId, me)).toBeNull();
  });

  it('收件箱里一条消息的编号带着房间和消息', () => {
    expect(messageAttentionId('!room:agent-room.test', 'mine')).toBe(
      'message:!room:agent-room.test:mine',
    );
  });
});
