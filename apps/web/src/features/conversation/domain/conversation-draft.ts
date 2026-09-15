import type { MessagePublicationDraft } from '@/features/messages/domain/publication';
import type { MessageRelation } from '@/features/messages/domain/message';
import { inspectPublicationRisks } from '@/features/messages/domain/publication';
import type { ConversationAttachment } from './conversation-attachment';

export function conversationDraft(
  text: string,
  mentions: readonly string[],
  relation?: MessageRelation,
  attachment?: ConversationAttachment,
): MessagePublicationDraft {
  text = text.trim().length === 0 && attachment !== undefined ? attachment.name : text;
  const summary = Array.from(text.trim().replace(/\s+/gu, ' ')).slice(0, 500).join('');
  return Object.freeze({
    body: attachment?.bytes ?? text,
    conversation: Object.freeze({
      text,
      mentions: Object.freeze([...mentions]),
      ...(attachment === undefined ? {} : { attachmentName: attachment.name }),
    }),
    mediaType: attachment?.mediaType ?? 'text/plain',
    riskFlags: inspectPublicationRisks(text),
    sensitivity: 'normal',
    summary,
    title: attachment?.name ?? Array.from(summary).slice(0, 120).join(''),
    ...(relation === undefined ? {} : { relation }),
  });
}
