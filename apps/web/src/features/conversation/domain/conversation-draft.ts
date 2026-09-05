import type { MessagePublicationDraft } from '@/features/messages/domain/publication';
import type { MessageRelation } from '@/features/messages/domain/message';
import { inspectPublicationRisks } from '@/features/messages/domain/publication';

export function conversationDraft(
  text: string,
  mentions: readonly string[],
  relation?: MessageRelation,
): MessagePublicationDraft {
  const summary = Array.from(text.trim().replace(/\s+/gu, ' ')).slice(0, 500).join('');
  return Object.freeze({
    body: text,
    conversation: Object.freeze({ text, mentions: Object.freeze([...mentions]) }),
    mediaType: 'text/plain',
    riskFlags: inspectPublicationRisks(text),
    sensitivity: 'normal',
    summary,
    title: Array.from(summary).slice(0, 120).join(''),
    ...(relation === undefined ? {} : { relation }),
  });
}
