import { describe, expect, it } from 'vitest';

import { mentionsAccount, type RoomMessageSignal } from './message';

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
