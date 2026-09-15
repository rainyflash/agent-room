import { describe, expect, it } from 'vitest';
import { conversationFixture } from '../testing/conversation-fixture';
import {
  conversationActorId,
  conversationTopic,
  emptyConversationFilter,
  searchConversation,
  validSearchDates,
} from './conversation-search';

describe('conversation search', () => {
  it('filters authorized active conversation text by person and local date inclusively', () => {
    const first = conversationFixture('one');
    const next = conversationFixture('two', {
      serverTimestamp: new Date('2026-09-15T12:00:00').getTime(),
    });
    const redacted = conversationFixture('removed', { lifecycle: 'redacted', preview: null });
    expect(
      searchConversation([redacted, next, first], {
        ...emptyConversationFilter,
        text: 'HELLO',
        actor: conversationActorId(first),
        from: '2026-09-14',
        until: '2026-09-14',
      }).map((message) => message.messageId),
    ).toEqual(['one']);
    expect(validSearchDates({ from: '2026-02-30', until: '' })).toBe(false);
    expect(validSearchDates({ from: '2026-09-15', until: '2026-09-14' })).toBe(false);
  });
  it('finds sibling and nested replies even if their parent is not loaded, and terminates on cycles', () => {
    const messages = [
      conversationFixture('one', { relation: { kind: 'reply', targetMessageId: 'missing' } }),
      conversationFixture('two', { relation: { kind: 'reply', targetMessageId: 'missing' } }),
      conversationFixture('three', { relation: { kind: 'reply', targetMessageId: 'two' } }),
      conversationFixture('other'),
    ];
    expect(
      searchConversation(messages, { ...emptyConversationFilter, topic: 'one' })
        .map((message) => message.messageId)
        .toSorted(),
    ).toEqual(['one', 'three', 'two']);
    expect(
      conversationTopic(
        [
          conversationFixture('a', { relation: { kind: 'reply', targetMessageId: 'b' } }),
          conversationFixture('b', { relation: { kind: 'reply', targetMessageId: 'a' } }),
        ],
        'a',
      ).size,
    ).toBe(2);
  });
});
