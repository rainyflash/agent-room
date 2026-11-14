import { describe, expect, it } from 'vitest';

import type { ConversationEntry } from '@/features/conversation/domain/conversation-entries';
import {
  betweenAgents,
  foldAgentRuns,
  type ConversationItem,
  type FoldContext,
} from '@/features/conversation/domain/conversation-folds';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomMessageSignal } from '@/features/messages/domain/message';

const YOU = '@you:agent-room.test';
const GRACE = '@grace:agent-room.test';
const ADA = '@ada:agent-room.test';
const BEN = '@ben:agent-room.test';
const CY = '@cy:agent-room.test';
const DAY = 24 * 60 * 60_000;
const NOON = new Date(2026, 10, 14, 12).getTime();

type Line = {
  readonly by: string;
  readonly mentions?: readonly string[];
  readonly everyone?: boolean;
  readonly replyTo?: string;
  readonly at?: number;
};

function line(id: string, index: number, input: Line): RoomMessageSignal {
  const agent = input.by === ADA || input.by === BEN || input.by === CY;
  const base = conversationFixture(id, {
    serverTimestamp: input.at ?? NOON + index * 60_000,
  });
  return {
    ...base,
    actor: agent
      ? {
          kind: 'agent',
          agentId: `agent-${input.by}`,
          instanceId: `instance-${input.by}`,
          displayName: input.by,
          matrixUserId: input.by,
          provenance: 'autonomous_agent',
        }
      : {
          kind: 'human',
          principalId: `principal-${input.by}`,
          displayName: input.by,
          matrixUserId: input.by,
          provenance: 'human',
        },
    preview: {
      title: 'Conversation',
      summary: id,
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
      conversation: { text: id, mentions: input.mentions ?? [] },
      ...(input.everyone === true ? { mentionsEveryone: true as const } : {}),
    },
    ...(input.replyTo === undefined
      ? {}
      : { relation: { kind: 'reply' as const, targetMessageId: input.replyTo } }),
  };
}

/** 一段对话：`[编号, 这一句]` 按先后。 */
function talk(lines: readonly (readonly [string, Line])[]) {
  const messages = lines.map(([id, input], index) => line(id, index, input));
  const context: FoldContext = {
    agents: new Set([ADA, BEN, CY]),
    byId: new Map(messages.map((message) => [message.messageId, message])),
  };
  const entries: ConversationEntry[] = messages.map((message) => ({ kind: 'message', message }));
  return { messages, context, entries };
}

/** 写成好比的样子：消息是它的编号，折起来的一串是 `[编号…]`。 */
function shape(items: readonly ConversationItem[]): readonly (string | readonly string[])[] {
  return items.map((item) =>
    item.kind === 'message'
      ? item.message.messageId
      : item.kind === 'notice'
        ? `notice:${item.change.matrixEventId}`
        : item.messages.map((message) => message.messageId),
  );
}

const agentRun = (prefix: string, count: number): readonly (readonly [string, Line])[] =>
  Array.from({ length: count }, (_, index) => [
    `${prefix}${String(index + 1)}`,
    { by: index % 2 === 0 ? ADA : BEN },
  ]);

describe('Agent 之间的长串折起来', () => {
  it('连着 4 句以上、后面还有别的话，才折成一项；3 句不折', () => {
    const four = talk([['q', { by: YOU }], ...agentRun('a', 4), ['end', { by: GRACE }]]);
    expect(shape(foldAgentRuns(four.entries, four.context))).toEqual([
      'q',
      ['a1', 'a2', 'a3', 'a4'],
      'end',
    ]);

    const three = talk([['q', { by: YOU }], ...agentRun('a', 3), ['end', { by: GRACE }]]);
    expect(shape(foldAgentRuns(three.entries, three.context))).toEqual([
      'q',
      'a1',
      'a2',
      'a3',
      'end',
    ]);
  });

  it('最下面还在进行的那一串不折，再长也不折', () => {
    const { entries, context } = talk([['q', { by: YOU }], ...agentRun('a', 9)]);
    expect(shape(foldAgentRuns(entries, context))).toEqual([
      'q',
      ...Array.from({ length: 9 }, (_, index) => `a${String(index + 1)}`),
    ]);
  });

  it('点了人、@了所有人、回复人说的话，都留在外面，把一串断成两段', () => {
    const { entries, context } = talk([
      ['q', { by: YOU }],
      ...agentRun('a', 4),
      ['to-you', { by: ADA, mentions: [YOU] }],
      ...agentRun('b', 4),
      ['to-all', { by: BEN, everyone: true }],
      ...agentRun('c', 4),
      ['re-you', { by: ADA, replyTo: 'q' }],
      ...agentRun('d', 2),
      ['end', { by: GRACE }],
    ]);
    expect(shape(foldAgentRuns(entries, context))).toEqual([
      'q',
      ['a1', 'a2', 'a3', 'a4'],
      'to-you',
      ['b1', 'b2', 'b3', 'b4'],
      'to-all',
      ['c1', 'c2', 'c3', 'c4'],
      're-you',
      'd1',
      'd2',
      'end',
    ]);
  });

  it('Agent 点 Agent、回复 Agent 说的话，还算 Agent 之间的', () => {
    const { entries, context } = talk([
      ['a1', { by: ADA }],
      ['a2', { by: BEN, mentions: [ADA] }],
      ['a3', { by: ADA, replyTo: 'a2' }],
      ['a4', { by: CY, mentions: [ADA, BEN] }],
      ['end', { by: YOU }],
    ]);
    const items = foldAgentRuns(entries, context);
    expect(shape(items)).toEqual([['a1', 'a2', 'a3', 'a4'], 'end']);
    const [fold] = items;
    if (fold?.kind !== 'fold') throw new Error('没折起来。');
    // 说过话的 Agent 按先后、不重复；拿最后一句当这一串的记号。
    expect(fold.speakers.map((speaker) => speaker.matrixUserId)).toEqual([ADA, BEN, CY]);
    expect(fold.key).toBe('a4');
  });

  it('拿不准的当成是找人的：点的 ID 不认识、被回复的那句没加载到', () => {
    const { messages, context } = talk([
      ['stranger', { by: ADA, mentions: ['@who:agent-room.test'] }],
      ['lost', { by: ADA, replyTo: 'not-loaded' }],
      ['plain', { by: ADA }],
      ['human', { by: GRACE }],
    ]);
    expect(messages.map((message) => betweenAgents(message, context))).toEqual([
      false,
      false,
      true,
      false,
    ]);
  });

  it('公告的改动夹在中间也断开', () => {
    const { entries, context } = talk([
      ...agentRun('a', 3),
      ...agentRun('b', 3),
      ['end', { by: YOU }],
    ]);
    const withNotice: ConversationEntry[] = [
      ...entries.slice(0, 3),
      {
        kind: 'notice',
        change: {
          kind: 'updated',
          matrixEventId: '$notice',
          serverTimestamp: NOON,
          updatedBy: GRACE,
        },
      },
      ...entries.slice(3),
    ];
    expect(shape(foldAgentRuns(withNotice, context))).toEqual([
      'a1',
      'a2',
      'a3',
      'notice:$notice',
      'b1',
      'b2',
      'b3',
      'end',
    ]);
    // 没有那一行改动时，这六句是连着的。
    expect(shape(foldAgentRuns(entries, context))).toEqual([
      ['a1', 'a2', 'a3', 'b1', 'b2', 'b3'],
      'end',
    ]);
  });

  it('一串不跨天：日期那一行不会被折进去', () => {
    const { entries, context } = talk([
      ...Array.from({ length: 5 }, (_, index): readonly [string, Line] => [
        `y${String(index + 1)}`,
        { by: index % 2 === 0 ? ADA : BEN, at: NOON - DAY + index * 60_000 },
      ]),
      ...Array.from({ length: 2 }, (_, index): readonly [string, Line] => [
        `t${String(index + 1)}`,
        { by: ADA, at: NOON + index * 60_000 },
      ]),
      ['end', { by: YOU, at: NOON + 10 * 60_000 }],
    ]);
    expect(shape(foldAgentRuns(entries, context))).toEqual([
      ['y1', 'y2', 'y3', 'y4', 'y5'],
      't1',
      't2',
      'end',
    ]);
  });

  it('往前补了更早的消息，这一串的记号不变', () => {
    const later = talk([...agentRun('a', 4), ['end', { by: YOU }]]);
    const earlier = talk([
      ['e1', { by: BEN, at: NOON - 2 * 60_000 }],
      ['e2', { by: ADA, at: NOON - 60_000 }],
      ...agentRun('a', 4),
      ['end', { by: YOU }],
    ]);
    const key = (items: readonly ConversationItem[]): string | undefined => {
      const [first] = items;
      return first?.kind === 'fold' ? first.key : undefined;
    };
    expect(key(foldAgentRuns(later.entries, later.context))).toBe('a4');
    expect(key(foldAgentRuns(earlier.entries, earlier.context))).toBe('a4');
  });
});
