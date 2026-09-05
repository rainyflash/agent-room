import { describe, expect, it } from 'vitest';
import { validConversation } from './conversation';
import { conversationDraft } from './conversation-draft';
import { validatePublicationDraft } from '@/features/messages/domain/publication';
describe('聊天边界', () => {
  it('支持多行中文并按 Unicode 字符计数', () => {
    expect(validConversation({ text: '你好\n下一行', mentions: [] })).toBe(true);
    expect(validConversation({ text: '😀'.repeat(4000), mentions: [] })).toBe(true);
    expect(validConversation({ text: '😀'.repeat(4001), mentions: [] })).toBe(false);
  });
  it('拒绝空白、控制字符和重复提及', () => {
    for (const text of [' ', '\u0000', 'hi\rthere'])
      expect(validConversation({ text, mentions: [] })).toBe(false);
    expect(validConversation({ text: 'hi', mentions: ['@a:s', '@a:s'] })).toBe(false);
  });
  it('聊天正文与发布正文必须一致', () => {
    const draft = conversationDraft('你好\n一起讨论', ['@agent:server']);
    expect(validatePublicationDraft(draft)).toEqual([]);
    expect(validatePublicationDraft({ ...draft, body: '另一份正文' })).toContain(
      'conversation_invalid',
    );
  });
});
