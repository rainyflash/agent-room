import { describe, expect, it } from 'vitest';
import { validConversation } from './conversation';
import { conversationDraft } from './conversation-draft';
import { validatePublicationDraft } from '@/features/messages/domain/publication';
import {
  attachmentIssue,
  maximumAttachmentBytes,
  imageAttachment,
} from './conversation-attachment';
describe('聊天边界', () => {
  it('附件保留独立的文件内容、说明和提及，也允许不填说明', () => {
    const file = { name: 'design.png', mediaType: 'image/png', bytes: new Uint8Array([0, 255, 4]) };
    const draft = conversationDraft('看一下设计', ['@agent:server'], undefined, file);
    expect(validatePublicationDraft(draft)).toEqual([]);
    expect(draft.body).toEqual(file.bytes);
    expect(draft.conversation).toEqual({
      text: '看一下设计',
      mentions: ['@agent:server'],
      attachmentName: 'design.png',
    });
    expect(validatePublicationDraft(conversationDraft('', [], undefined, file))).toEqual([]);
    expect(validatePublicationDraft({ ...draft, body: 'This is not the file' })).toContain(
      'conversation_invalid',
    );
  });
  it('拒绝空文件、超限文件和路径，SVG 不会内联执行', () => {
    const file = { name: 'design.png', mediaType: 'image/png', bytes: new Uint8Array([1]) };
    expect(attachmentIssue({ ...file, bytes: new Uint8Array() })).toBe('empty');
    expect(attachmentIssue({ ...file, bytes: new Uint8Array(maximumAttachmentBytes + 1) })).toBe(
      'tooLarge',
    );
    expect(attachmentIssue({ ...file, name: '../design.png' })).toBe('invalid');
    expect(imageAttachment('image/svg+xml')).toBe(false);
    expect(imageAttachment('image/png')).toBe(true);
  });
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
