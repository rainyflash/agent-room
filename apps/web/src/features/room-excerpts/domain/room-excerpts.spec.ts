import { describe, expect, it } from 'vitest';

import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { MessagePreview, RoomMessageSignal } from '@/features/messages/domain/message';

import {
  EXCERPT_MAX_MESSAGES,
  EXCERPT_PREVIEW_CHARS,
  excerptFailureKind,
  excerptFailureRetryable,
  excerptMessages,
  excerptPlaces,
  excerptPreviewLine,
  excerptRetryMinutes,
  excerptShareable,
  pickExcerptMessage,
  roomExcerptListSchema,
  roomExcerptSchema,
  sharedExcerptSchema,
} from './room-excerpts';

const LIVE_SINCE = new Date('2026-11-16T09:00:00Z').getTime();

/** 换掉预览里的几样，别的照一句普通的聊天消息。 */
function withPreview(id: string, preview: Partial<MessagePreview>): RoomMessageSignal {
  return conversationFixture(id, {
    preview: {
      title: 'Conversation',
      summary: id,
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
      conversation: { text: 'Hello', mentions: [] },
      ...preview,
    },
  });
}

/** 第 1 到第 n 句，开实况以后每分钟一句，ID 是 `m1`…`mn`。 */
function timeline(count: number): readonly RoomMessageSignal[] {
  return Array.from({ length: count }, (_, index) =>
    conversationFixture(`m${String(index + 1)}`, {
      serverTimestamp: LIVE_SINCE + (index + 1) * 60_000,
    }),
  );
}

function ids(messages: readonly RoomMessageSignal[] | null): readonly string[] | null {
  return messages === null ? null : messages.map((message) => message.messageId);
}

describe('能不能分享这一句', () => {
  const said = conversationFixture('a', { serverTimestamp: LIVE_SINCE + 1 });

  it('开了实况以后真发出去的聊天消息能分享；正好在开的那一刻也算', () => {
    expect(excerptShareable(said, LIVE_SINCE)).toBe(true);
    expect(excerptShareable({ ...said, serverTimestamp: LIVE_SINCE }, LIVE_SINCE)).toBe(true);
  });

  it('开实况以前说的、还在发的本地回显、撤回和隐藏的、系统那几行都不能', () => {
    expect(excerptShareable({ ...said, serverTimestamp: LIVE_SINCE - 1 }, LIVE_SINCE)).toBe(false);
    expect(excerptShareable({ ...said, matrixEventId: '~local-1' }, LIVE_SINCE)).toBe(false);
    expect(excerptShareable({ ...said, lifecycle: 'redacted' }, LIVE_SINCE)).toBe(false);
    expect(excerptShareable({ ...said, lifecycle: 'moderated' }, LIVE_SINCE)).toBe(false);
    // 系统那几行（置顶、改须知）没有聊天正文。
    const pinned = conversationFixture('n', {
      serverTimestamp: LIVE_SINCE + 1,
      preview: {
        title: 'Room notice',
        summary: 'Pinned a message',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
      },
    });
    expect(excerptShareable(pinned, LIVE_SINCE)).toBe(false);
  });
});

describe('选一段', () => {
  const messages = timeline(40);
  const only = (id: string) => ({ startId: id, endId: id });

  it('点后面的一句是结尾；点中间的一句缩到那儿；点开头只留它自己', () => {
    const grown = pickExcerptMessage(messages, only('m5'), 'm10');
    expect(grown).toEqual({ range: { startId: 'm5', endId: 'm10' }, tooLong: false });
    expect(ids(excerptMessages(messages, grown.range))).toEqual([
      'm5',
      'm6',
      'm7',
      'm8',
      'm9',
      'm10',
    ]);
    expect(pickExcerptMessage(messages, grown.range, 'm7').range).toEqual({
      startId: 'm5',
      endId: 'm7',
    });
    expect(pickExcerptMessage(messages, grown.range, 'm5').range).toEqual(only('m5'));
  });

  it('点开头前面的一句：它成了开头，结尾不动', () => {
    expect(pickExcerptMessage(messages, { startId: 'm5', endId: 'm10' }, 'm2')).toEqual({
      range: { startId: 'm2', endId: 'm10' },
      tooLong: false,
    });
  });

  it(`正好 ${String(EXCERPT_MAX_MESSAGES)} 句可以，多一句就不改、说为什么`, () => {
    const full = pickExcerptMessage(messages, only('m1'), 'm30');
    expect(full.tooLong).toBe(false);
    expect(excerptMessages(messages, full.range)).toHaveLength(EXCERPT_MAX_MESSAGES);

    expect(pickExcerptMessage(messages, only('m1'), 'm31')).toEqual({
      range: only('m1'),
      tooLong: true,
    });
    // 往前挪开头也一样：第 2 到第 40 句是 39 句。
    expect(pickExcerptMessage(messages, { startId: 'm11', endId: 'm40' }, 'm2')).toEqual({
      range: { startId: 'm11', endId: 'm40' },
      tooLong: true,
    });
  });

  it('开头或结尾已经不在对话里（撤回了）时选的不算数，点了也不动', () => {
    const gone = { startId: 'gone', endId: 'm3' };
    expect(excerptMessages(messages, gone)).toBeNull();
    expect(excerptMessages(messages, { startId: 'm5', endId: 'm2' })).toBeNull();
    expect(pickExcerptMessage(messages, gone, 'm4')).toEqual({ range: gone, tooLong: false });
    expect(pickExcerptMessage(messages, only('m1'), 'nowhere')).toEqual({
      range: only('m1'),
      tooLong: false,
    });
  });

  it('选中的每一句排在哪：头一句、中间、最后一句；只有一句时是“只有它”', () => {
    const places = excerptPlaces(messages.slice(0, 3));
    expect([...places.entries()]).toEqual([
      ['m1', 'first'],
      ['m2', 'middle'],
      ['m3', 'last'],
    ]);
    expect([...excerptPlaces(messages.slice(4, 5)).entries()]).toEqual([['m5', 'only']]);
  });
});

describe('预览里一句的样子', () => {
  it('不公开的只说一句', () => {
    for (const sensitivity of ['sensitive', 'restricted'] as const) {
      expect(excerptPreviewLine(withPreview('s', { sensitivity }))).toEqual({ kind: 'withheld' });
    }
  });

  it(`正文给开头 ${String(EXCERPT_PREVIEW_CHARS)} 个字，按字算不按字节`, () => {
    const text = '说'.repeat(EXCERPT_PREVIEW_CHARS + 5);
    const line = excerptPreviewLine(withPreview('long', { conversation: { text, mentions: [] } }));
    expect(line).toEqual({
      kind: 'text',
      text: '说'.repeat(EXCERPT_PREVIEW_CHARS),
      truncated: true,
      attachment: false,
    });
    expect(excerptPreviewLine(withPreview('short', {}))).toEqual({
      kind: 'text',
      text: 'Hello',
      truncated: false,
      attachment: false,
    });
  });

  it('附件只说有；只发了一个文件时不把文件名当正文', () => {
    const withFile = (text: string) =>
      withPreview('file', { conversation: { text, attachmentName: 'plan.pdf', mentions: [] } });
    expect(excerptPreviewLine(withFile('plan.pdf'))).toEqual({
      kind: 'text',
      text: '',
      truncated: false,
      attachment: true,
    });
    expect(excerptPreviewLine(withFile('Here is the plan'))).toEqual({
      kind: 'text',
      text: 'Here is the plan',
      truncated: false,
      attachment: true,
    });
  });
});

describe('分享没成时是哪一种', () => {
  it('错误码照控制面的认；选的不对时看 reason 分开开实况以前和找不到', () => {
    const kind = (code: string, reason?: string) =>
      excerptFailureKind({ code, retryable: false, ...(reason === undefined ? {} : { reason }) });
    expect(kind('room_excerpt.not_live')).toBe('not_live');
    expect(kind('room_excerpt.too_long')).toBe('too_long');
    expect(kind('room_excerpt.invalid_range', 'before_live')).toBe('before_live');
    expect(kind('room_excerpt.invalid_range', 'not_found')).toBe('range_not_found');
    expect(kind('room_excerpt.invalid_range')).toBe('range_not_found');
    expect(kind('room_excerpt.rate_limited')).toBe('rate_limited');
    expect(kind('room_excerpt.not_found')).toBe('not_found');
    expect(kind('room_excerpt.forbidden')).toBe('forbidden');
    expect(kind('room_excerpt.dependency_unavailable')).toBe('unavailable');
    expect(kind('room_excerpt.internal')).toBe('unavailable');
    expect(kind('room_excerpt.invalid')).toBe('invalid');
    expect(kind('room_excerpt.invalid_resource')).toBe('invalid');
    expect(kind('private_room.invalid_response')).toBe('invalid');
    expect(kind('room_excerpt.invalid_origin')).toBe('invalid_origin');
    expect(kind('private_room.unreachable')).toBe('unreachable');
    expect(kind('something.new')).toBe('other');
  });

  it('只有等一会儿再试有用的给“再试一次”：太多了要等，选的不对要改', () => {
    expect(excerptFailureRetryable('unavailable')).toBe(true);
    expect(excerptFailureRetryable('unreachable')).toBe(true);
    expect(excerptFailureRetryable('other')).toBe(true);
    for (const kind of [
      'rate_limited',
      'before_live',
      'too_long',
      'not_live',
      'forbidden',
    ] as const)
      expect(excerptFailureRetryable(kind)).toBe(false);
  });

  it('还要等几分钟向上取整，最少 1 分钟；服务器没说就不说', () => {
    const wait = (seconds?: number) =>
      excerptRetryMinutes({
        code: 'room_excerpt.rate_limited',
        retryable: true,
        ...(seconds === undefined ? {} : { retryAfterSeconds: seconds }),
      });
    expect(wait(1200)).toBe(20);
    expect(wait(61)).toBe(2);
    expect(wait(0)).toBe(1);
    expect(wait()).toBeNull();
  });
});

describe('回答严格校验', () => {
  const excerpt = {
    code: 'Kq7Lm2Zp',
    url: 'https://agentroom.chat/r/aB3xYz/s/Kq7Lm2Zp',
    messageCount: 4,
    firstMessageAtUnixMs: LIVE_SINCE,
    sharedAtUnixMs: LIVE_SINCE + 60_000,
    mine: true,
  };

  it('照控制面的形状读得出来；房间没有分享码时链接是 null', () => {
    expect(roomExcerptSchema.parse(excerpt)).toEqual(excerpt);
    expect(roomExcerptSchema.parse({ ...excerpt, url: null }).url).toBeNull();
    expect(roomExcerptListSchema.parse({ excerpts: [excerpt] }).excerpts).toHaveLength(1);
    expect(sharedExcerptSchema.parse({ code: excerpt.code, url: excerpt.url })).toEqual({
      code: excerpt.code,
      url: excerpt.url,
    });
  });

  it('多字段、段码不像样、句数超出、不是网址都不认', () => {
    for (const body of [
      { ...excerpt, sharedBy: 'someone' },
      { ...excerpt, code: 'short' },
      { ...excerpt, code: 'Kq7Lm2Z!' },
      { ...excerpt, messageCount: 0 },
      { ...excerpt, messageCount: EXCERPT_MAX_MESSAGES + 1 },
      { ...excerpt, url: 'javascript:alert(1)' },
      { ...excerpt, mine: undefined },
    ])
      expect(roomExcerptSchema.safeParse(body).success).toBe(false);
    expect(
      sharedExcerptSchema.safeParse({ code: excerpt.code, url: excerpt.url, extra: 1 }).success,
    ).toBe(false);
  });
});
